//! Quota state and the rate-limiting algorithm: per-key bucket, token-bucket
//! and GCRA evaluation, IETF `RateLimit-*` headers, key extraction, and the
//! per-request middleware handler.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use http::HeaderValue;
use http::header::RETRY_AFTER;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Algorithm;
use super::config::Config;
use super::config::KeyFn;
use super::config::UnkeyedBehavior;

#[derive(Clone)]
pub(crate) struct Bucket {
  available: f64,
  pub(crate) last_refill: Instant,
}

fn default_key(req: &Request) -> Option<String> {
  if let Some(info) = req.extensions().get::<ConnInfo>()
    && let PeerAddr::Ip(sa) = &info.peer
  {
    return Some(format!("ip:{}", sa.ip()));
  }
  if let Some(sa) = req.extensions().get::<SocketAddr>() {
    return Some(format!("ip:{}", sa.ip()));
  }
  None
}

struct Outcome {
  allowed: bool,
  remaining: u32,
  reset_secs: u64,
  retry_after_secs: u64,
}

fn evaluate(cfg: &Config, bucket: &mut Bucket, now: Instant) -> Outcome {
  let cap = f64::from(cfg.max_requests);
  match cfg.algorithm {
    Algorithm::TokenBucket => {
      // Lazy refill so each request observes the latest count even between
      // ticker ticks.
      let dt = now
        .duration_since(bucket.last_refill)
        .as_secs_f64()
        .max(0.0);
      let rate_per_sec = f64::from(cfg.refill_rate) / (cfg.refill_interval_ms as f64 / 1_000.0);
      bucket.available = (bucket.available + dt * rate_per_sec).min(cap);
      bucket.last_refill = now;
      let allowed = bucket.available >= 1.0;
      if allowed {
        bucket.available -= 1.0;
      }
      let remaining = bucket.available.max(0.0).floor() as u32;
      let needed = (1.0 - bucket.available).max(0.0);
      let reset_secs = if rate_per_sec > 0.0 {
        (needed / rate_per_sec).ceil() as u64
      } else {
        0
      };
      let retry_after_secs = if allowed { 0 } else { reset_secs.max(1) };
      Outcome {
        allowed,
        remaining,
        reset_secs,
        retry_after_secs,
      }
    }
    Algorithm::Gcra => {
      // GCRA: maintain a virtual "next free time"; if it is in the future
      // beyond the burst tolerance, reject. We map `available` ↔ remaining
      // headroom for backwards-compatible book-keeping.
      let rate_per_sec = f64::from(cfg.refill_rate) / (cfg.refill_interval_ms as f64 / 1_000.0);
      let increment = if rate_per_sec > 0.0 {
        1.0 / rate_per_sec
      } else {
        f64::INFINITY
      };
      let burst_tolerance = cap * increment;
      // bucket.available represents seconds of "credit" remaining (negative
      // means the request would have to wait).
      let elapsed = now
        .duration_since(bucket.last_refill)
        .as_secs_f64()
        .max(0.0);
      bucket.available = (bucket.available - elapsed).max(0.0);
      bucket.last_refill = now;
      let allowed = bucket.available + increment <= burst_tolerance;
      if allowed {
        bucket.available += increment;
      }
      let credit_used = bucket.available;
      let remaining = ((burst_tolerance - credit_used).max(0.0) * rate_per_sec).floor() as u32;
      let reset_secs = bucket.available.ceil() as u64;
      let retry_after_secs = if allowed {
        0
      } else {
        ((bucket.available + increment - burst_tolerance).max(0.0)).ceil() as u64
      };
      Outcome {
        allowed,
        remaining,
        reset_secs,
        retry_after_secs: retry_after_secs.max(1),
      }
    }
  }
}

/// Write the IETF draft-`RateLimit-Headers` set into the response.
///
/// PPL-16: previously this used `headers.insert(...)` which replaces any
/// existing value. In composed middleware setups where multiple
/// rate-limiters share the response, the outer (last-to-run) limiter
/// silently clobbered the inner limiter's `ratelimit-*` headers. The most
/// informative signal — typically the innermost limiter's
/// `ratelimit-remaining: 0` rejection — could be lost on its way back to
/// the client.
///
/// Switch to `entry(...).or_insert(...)` so the FIRST limiter to write the
/// header wins. In middleware chains the inner (closest-to-handler) limiter
/// runs its post-processing FIRST on the response path, so first-wins is
/// inner-wins — which is the more restrictive observable signal.
fn write_rate_limit_headers(headers: &mut http::HeaderMap, cfg: &Config, outcome: &Outcome) {
  if let Ok(v) = HeaderValue::from_str(&cfg.max_requests.to_string()) {
    headers.entry("ratelimit-limit").or_insert(v);
  }
  if let Ok(v) = HeaderValue::from_str(&outcome.remaining.to_string()) {
    headers.entry("ratelimit-remaining").or_insert(v);
  }
  if let Ok(v) = HeaderValue::from_str(&outcome.reset_secs.to_string()) {
    headers.entry("ratelimit-reset").or_insert(v);
  }
}

pub(crate) async fn handle(
  req: Request,
  next: Next,
  cfg: Config,
  store: Arc<SccHashMap<String, Mutex<Bucket>>>,
  key_fn: Option<KeyFn>,
) -> Response {
  let key = match key_fn.as_ref() {
    Some(f) => f(&req),
    None => default_key(&req),
  };
  let Some(key) = key else {
    return match cfg.on_unkeyed {
      UnkeyedBehavior::Allow => next.run(req).await,
      UnkeyedBehavior::Reject => http::Response::builder()
        .status(cfg.status_on_limit)
        .body(TakoBody::empty())
        .expect("valid rate-limit response"),
    };
  };

  let outcome = {
    let entry = store.entry_async(key).await.or_insert_with(|| {
      Mutex::new(Bucket {
        available: f64::from(cfg.max_requests),
        last_refill: Instant::now(),
      })
    });
    // `parking_lot::Mutex` (sync lock) is deliberate here: we hold it across
    // a strictly synchronous `evaluate` call and never `.await` under the
    // guard. A `tokio::sync::Mutex` would force this hot path through a
    // Notify-backed wait list with no contention benefit, and would prevent
    // running the limiter outside an async runtime context.
    let mut bucket = entry.get().lock();
    evaluate(&cfg, &mut bucket, Instant::now())
  };

  if !outcome.allowed {
    let mut resp = http::Response::builder()
      .status(cfg.status_on_limit)
      .body(TakoBody::empty())
      .expect("valid rate-limit response");
    write_rate_limit_headers(resp.headers_mut(), &cfg, &outcome);
    if let Ok(v) = HeaderValue::from_str(&outcome.retry_after_secs.to_string()) {
      resp.headers_mut().insert(RETRY_AFTER, v);
    }
    return resp;
  }

  let mut resp = next.run(req).await;
  write_rate_limit_headers(resp.headers_mut(), &cfg, &outcome);
  resp
}
