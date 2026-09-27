//! Quota state and the rate-limiting algorithm: per-key bucket, token-bucket
//! and GCRA evaluation, IETF `RateLimit-*` headers, key extraction, and the
//! per-request middleware handler.

use std::sync::Arc;
use std::time::Instant;

use http::HeaderValue;
use http::header::RETRY_AFTER;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::config::Algorithm;
use super::config::Config;
use super::config::KeyFn;
use super::config::UnkeyedBehavior;
use super::key::BucketKey;
use super::key::default_key;

#[derive(Clone)]
pub(crate) struct Bucket {
  available: f64,
  pub(crate) last_refill: Instant,
}

struct Outcome {
  limit: u32,
  allowed: bool,
  remaining: u32,
  reset_secs: u64,
  retry_after_secs: u64,
}

fn evaluate(cfg: &Config, bucket: &mut Bucket, now: Instant) -> Outcome {
  let cap = f64::from(cfg.max_requests);
  match cfg.algorithm {
    Algorithm::TokenBucket => {
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
        limit: cfg.max_requests,
        allowed,
        remaining,
        reset_secs,
        retry_after_secs,
      }
    }
    Algorithm::Gcra => {
      // GCRA stores time debt in seconds, rather than the token bucket's token count.
      let rate_per_sec = f64::from(cfg.refill_rate) / (cfg.refill_interval_ms as f64 / 1_000.0);
      let increment = if rate_per_sec > 0.0 {
        1.0 / rate_per_sec
      } else {
        f64::INFINITY
      };
      let burst_tolerance = cap * increment;
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
        limit: cfg.max_requests,
        allowed,
        remaining,
        reset_secs,
        retry_after_secs: retry_after_secs.max(1),
      }
    }
  }
}

fn write_rate_limit_headers(headers: &mut http::HeaderMap, outcome: &Outcome) {
  // Inner middleware decisions survive outer limiters on the response path.
  headers
    .entry("ratelimit-limit")
    .or_insert(HeaderValue::from(outcome.limit));
  headers
    .entry("ratelimit-remaining")
    .or_insert(HeaderValue::from(outcome.remaining));
  headers
    .entry("ratelimit-reset")
    .or_insert(HeaderValue::from(outcome.reset_secs));
}

pub(crate) async fn handle(
  req: Request,
  next: Next,
  cfg: Config,
  store: Arc<SccHashMap<BucketKey, Mutex<Bucket>>>,
  key_fn: Option<KeyFn>,
  backend: Option<Arc<dyn crate::stores::RateLimitStore>>,
) -> Response {
  let key = match key_fn.as_ref() {
    Some(f) => f(&req).map(BucketKey::Custom),
    None => default_key(&req, &cfg),
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

  let outcome = if let Some(backend) = backend {
    let decision = match backend.consume(&key.storage_key(), 1).await {
      Ok(decision) => decision,
      Err(error) => return crate::stores::runtime::unavailable(error),
    };
    let (allowed, snapshot) = match decision {
      Ok(snapshot) => (true, snapshot),
      Err(snapshot) => (false, snapshot),
    };
    Outcome {
      limit: snapshot.limit,
      allowed,
      remaining: snapshot.remaining,
      reset_secs: snapshot.reset_secs,
      retry_after_secs: snapshot.retry_after_secs,
    }
  } else {
    let entry = store.entry_async(key).await.or_insert_with(|| {
      Mutex::new(Bucket {
        available: match cfg.algorithm {
          Algorithm::TokenBucket => f64::from(cfg.max_requests),
          Algorithm::Gcra => 0.0,
        },
        last_refill: Instant::now(),
      })
    });
    // The synchronous quota update never holds this guard across an await.
    let mut bucket = entry.get().lock();
    evaluate(&cfg, &mut bucket, Instant::now())
  };

  if !outcome.allowed {
    let mut resp = http::Response::builder()
      .status(cfg.status_on_limit)
      .body(TakoBody::empty())
      .expect("valid rate-limit response");
    write_rate_limit_headers(resp.headers_mut(), &outcome);
    resp
      .headers_mut()
      .insert(RETRY_AFTER, HeaderValue::from(outcome.retry_after_secs));
    return resp;
  }

  let mut resp = next.run(req).await;
  write_rate_limit_headers(resp.headers_mut(), &outcome);
  resp
}
