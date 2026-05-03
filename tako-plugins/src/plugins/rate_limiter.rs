#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Rate limiting plugin: token-bucket or GCRA, with composite keys and
//! IETF rate-limit response headers.
//!
//! v2 additions over the original token-bucket-by-IP design:
//!
//! - **Composite keys.** Default key is still the peer IP, but
//!   [`RateLimiterBuilder::key_fn`] lets callers compose per-route /
//!   per-tenant / per-user buckets without forking the plugin.
//! - **Strict IP fallback.** Requests without a discoverable peer IP no
//!   longer all collapse into the `0.0.0.0` bucket — the request is treated
//!   as unkeyed and skipped (configurable via [`RateLimiterBuilder::on_unkeyed`]).
//! - **`RateLimit-*` headers.** Emits `RateLimit-Limit`, `RateLimit-Remaining`,
//!   `RateLimit-Reset`, and `Retry-After` per the IETF httpapi draft.
//! - **GCRA mode.** Opt in via [`Algorithm::Gcra`]. The per-key state stays
//!   one f64; no separate refill ticker.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use http::HeaderValue;
use http::StatusCode;
use http::header::RETRY_AFTER;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tako_core::body::TakoBody;
use tako_core::conn_info::ConnInfo;
use tako_core::conn_info::PeerAddr;
use tako_core::middleware::Next;
use tako_core::plugins::TakoPlugin;
use tako_core::router::Router;
use tako_core::types::Request;
use tako_core::types::Response;

/// Rate-limiting algorithm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
  /// Classic token bucket. `refill_rate` tokens added every
  /// `refill_interval_ms`, capped at `max_requests` (burst capacity).
  TokenBucket,
  /// Generic Cell Rate Algorithm (RFC 4341 / IETF rate-limit headers draft).
  /// One token every `1 / rate_per_second` second; bursts up to
  /// `max_requests` allowed.
  Gcra,
}

/// Behavior when a request cannot be keyed (unknown peer, custom key fn
/// returned `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnkeyedBehavior {
  /// Allow the request through without rate-limit accounting.
  Allow,
  /// Reject with the configured `status_on_limit`.
  Reject,
}

/// Configuration parameters.
#[derive(Clone)]
pub struct Config {
  /// Maximum burst capacity.
  pub max_requests: u32,
  /// Tokens added per refill interval (TokenBucket only).
  pub refill_rate: u32,
  /// Refill interval (TokenBucket only).
  pub refill_interval_ms: u64,
  /// HTTP status returned on rejection.
  pub status_on_limit: StatusCode,
  /// Algorithm choice.
  pub algorithm: Algorithm,
  /// Behavior for requests that cannot be keyed.
  pub on_unkeyed: UnkeyedBehavior,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      max_requests: 60,
      refill_rate: 60,
      refill_interval_ms: 1_000,
      status_on_limit: StatusCode::TOO_MANY_REQUESTS,
      algorithm: Algorithm::TokenBucket,
      on_unkeyed: UnkeyedBehavior::Allow,
    }
  }
}

/// Custom key function: maps a request to a rate-limit bucket id. Returning
/// `None` defers to [`Config::on_unkeyed`].
pub type KeyFn = Arc<dyn Fn(&Request) -> Option<String> + Send + Sync + 'static>;

/// Builder.
pub struct RateLimiterBuilder {
  cfg: Config,
  key_fn: Option<KeyFn>,
}

impl Default for RateLimiterBuilder {
  fn default() -> Self {
    Self::new()
  }
}

impl RateLimiterBuilder {
  pub fn new() -> Self {
    Self {
      cfg: Config::default(),
      key_fn: None,
    }
  }

  pub fn max_requests(mut self, n: u32) -> Self {
    self.cfg.max_requests = n;
    self
  }

  pub fn refill_rate(mut self, n: u32) -> Self {
    self.cfg.refill_rate = n;
    self
  }

  pub fn refill_interval_ms(mut self, ms: u64) -> Self {
    self.cfg.refill_interval_ms = ms.max(1);
    self
  }

  pub fn status(mut self, st: StatusCode) -> Self {
    self.cfg.status_on_limit = st;
    self
  }

  pub fn algorithm(mut self, a: Algorithm) -> Self {
    self.cfg.algorithm = a;
    self
  }

  pub fn on_unkeyed(mut self, b: UnkeyedBehavior) -> Self {
    self.cfg.on_unkeyed = b;
    self
  }

  /// Override the bucket key. Common compositions:
  /// `format!("{}|{}", path, ip)` for per-route+IP buckets,
  /// `Some(req.headers().get("x-tenant-id")?.to_str().ok()?.to_string())`
  /// for per-tenant.
  pub fn key_fn<F>(mut self, f: F) -> Self
  where
    F: Fn(&Request) -> Option<String> + Send + Sync + 'static,
  {
    self.key_fn = Some(Arc::new(f));
    self
  }

  /// Convenience: N requests / second.
  pub fn requests_per_second(mut self, n: u32) -> Self {
    self.cfg.max_requests = n;
    self.cfg.refill_rate = n;
    self.cfg.refill_interval_ms = 1_000;
    self
  }

  /// Convenience: N requests / minute.
  pub fn requests_per_minute(mut self, n: u32) -> Self {
    self.cfg.max_requests = n;
    self.cfg.refill_rate = n;
    self.cfg.refill_interval_ms = 60_000;
    self
  }

  pub fn build(self) -> RateLimiterPlugin {
    RateLimiterPlugin {
      cfg: self.cfg,
      key_fn: self.key_fn,
      store: Arc::new(SccHashMap::new()),
      task_started: Arc::new(AtomicBool::new(false)),
    }
  }
}

#[derive(Clone)]
struct Bucket {
  available: f64,
  last_refill: Instant,
}

#[derive(Clone)]
#[doc(alias = "rate_limiter")]
#[doc(alias = "ratelimit")]
pub struct RateLimiterPlugin {
  cfg: Config,
  key_fn: Option<KeyFn>,
  store: Arc<SccHashMap<String, Mutex<Bucket>>>,
  task_started: Arc<AtomicBool>,
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

impl TakoPlugin for RateLimiterPlugin {
  fn name(&self) -> &'static str {
    "RateLimiterPlugin"
  }

  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = self.cfg.clone();
    let store = self.store.clone();
    let key_fn = self.key_fn.clone();

    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      let store = store.clone();
      let key_fn = key_fn.clone();
      async move { handle(req, next, cfg, store, key_fn).await }
    });

    if matches!(self.cfg.algorithm, Algorithm::TokenBucket)
      && !self.task_started.swap(true, Ordering::SeqCst)
    {
      let cfg = self.cfg.clone();
      let store = self.store.clone();

      let purge_after = Duration::from_secs(300);
      let interval = Duration::from_millis(cfg.refill_interval_ms);
      let refill_amount = cfg.refill_rate as f64;
      let cap = cfg.max_requests as f64;

      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
          tick.tick().await;
          let now = Instant::now();
          store
            .retain_async(|_, mutex| {
              let mut bucket = mutex.lock();
              bucket.available = (bucket.available + refill_amount).min(cap);
              bucket.last_refill = now;
              now.duration_since(bucket.last_refill) < purge_after
            })
            .await;
        }
      });

      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        loop {
          compio::time::sleep(interval).await;
          let now = Instant::now();
          store
            .retain_async(|_, mutex| {
              let mut bucket = mutex.lock();
              bucket.available = (bucket.available + refill_amount).min(cap);
              bucket.last_refill = now;
              now.duration_since(bucket.last_refill) < purge_after
            })
            .await;
        }
      })
      .detach();
    }

    Ok(())
  }
}

struct Outcome {
  allowed: bool,
  remaining: u32,
  reset_secs: u64,
  retry_after_secs: u64,
}

fn evaluate(cfg: &Config, bucket: &mut Bucket, now: Instant) -> Outcome {
  let cap = cfg.max_requests as f64;
  match cfg.algorithm {
    Algorithm::TokenBucket => {
      // Lazy refill so each request observes the latest count even between
      // ticker ticks.
      let dt = now
        .duration_since(bucket.last_refill)
        .as_secs_f64()
        .max(0.0);
      let rate_per_sec = cfg.refill_rate as f64 / (cfg.refill_interval_ms as f64 / 1_000.0);
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
      let rate_per_sec = cfg.refill_rate as f64 / (cfg.refill_interval_ms as f64 / 1_000.0);
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

fn write_rate_limit_headers(headers: &mut http::HeaderMap, cfg: &Config, outcome: &Outcome) {
  if let Ok(v) = HeaderValue::from_str(&cfg.max_requests.to_string()) {
    headers.insert("ratelimit-limit", v);
  }
  if let Ok(v) = HeaderValue::from_str(&outcome.remaining.to_string()) {
    headers.insert("ratelimit-remaining", v);
  }
  if let Ok(v) = HeaderValue::from_str(&outcome.reset_secs.to_string()) {
    headers.insert("ratelimit-reset", v);
  }
}

async fn handle(
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
  let key = match key {
    Some(k) => k,
    None => match cfg.on_unkeyed {
      UnkeyedBehavior::Allow => return next.run(req).await,
      UnkeyedBehavior::Reject => {
        return http::Response::builder()
          .status(cfg.status_on_limit)
          .body(TakoBody::empty())
          .expect("valid rate-limit response");
      }
    },
  };

  let outcome = {
    let entry = store.entry_async(key).await.or_insert_with(|| {
      Mutex::new(Bucket {
        available: cfg.max_requests as f64,
        last_refill: Instant::now(),
      })
    });
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
