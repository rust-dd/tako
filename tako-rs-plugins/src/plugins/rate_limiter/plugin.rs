//! The rate-limiter plugin: fluent builder, the plugin struct, and the
//! [`TakoPlugin`] wiring that installs the middleware and the staleness
//! janitor.

use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use http::StatusCode;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

use super::algorithm::Bucket;
use super::algorithm::handle;
use super::config::Algorithm;
use super::config::Config;
use super::config::KeyFn;
use super::config::UnkeyedBehavior;

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

  /// Build the plugin.
  ///
  /// # Panics
  ///
  /// Panics if `refill_rate == 0`. A zero rate poisons the GCRA path
  /// (`rate_per_sec=0` → division-by-zero → `INFINITY`/`NaN` arithmetic that
  /// silently bypasses the limiter) and is also nonsensical for the token
  /// bucket (the bucket would never refill). Use a deliberately tiny rate
  /// like `1` with a long `refill_interval` if you want hard throttling.
  ///
  /// Also panics if `max_requests == 0`. With cap zero, the token bucket's
  /// `available >= 1.0` check fails forever and GCRA's `burst_tolerance=0`
  /// produces the same result — every request is denied silently with no
  /// startup signal that the limiter is essentially a hard-deny gate.
  pub fn build(self) -> RateLimiterPlugin {
    assert!(
      self.cfg.refill_rate > 0,
      "RateLimiter::refill_rate must be > 0 (zero rate produces INFINITY in GCRA)"
    );
    assert!(
      self.cfg.refill_interval_ms > 0,
      "RateLimiter::refill_interval_ms must be > 0 (zero interval is divide-by-zero)"
    );
    // PPL-07: catch the "all-denied" misconfiguration at build time instead
    // of silently 429-ing every request at runtime. Symmetry with the two
    // asserts above.
    assert!(
      self.cfg.max_requests > 0,
      "RateLimiter::max_requests must be > 0 (zero cap silently denies every request)"
    );
    RateLimiterPlugin {
      cfg: self.cfg,
      key_fn: self.key_fn,
      store: Arc::new(SccHashMap::new()),
      task_started: Arc::new(AtomicBool::new(false)),
    }
  }
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

      // Janitor is **staleness-eviction only**. Refilling here too would
      // double-count: `evaluate()` already does lazy refill per request
      // (`dt * rate_per_sec` from the last observed timestamp), so an
      // eager refill in the janitor on top would push the effective rate
      // toward 2× the configured value — a silent weakening of the
      // DoS-quota control. We also can't mutate `last_refill` here
      // because doing so before the staleness predicate makes
      // `duration_since` always 0 and turns `purge_after` into dead code.
      let purge_after = Duration::from_secs(300);
      let interval = Duration::from_millis(cfg.refill_interval_ms);

      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
          tick.tick().await;
          let now = Instant::now();
          store
            .retain_async(|_, mutex| {
              let bucket = mutex.lock();
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
              let bucket = mutex.lock();
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
