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
use super::key::BucketKey;

#[cfg(test)]
mod tests;

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

  /// Resolve the client IP using the router's trusted-proxy `IpAddrConfig`.
  /// Without trusted proxies this still uses the transport peer. Defaults to false.
  pub fn client_ip(mut self, enabled: bool) -> Self {
    self.cfg.client_ip = enabled;
    self
  }

  /// Group IPv6 clients by a prefix; 64 groups a subnet and 128 keeps individual addresses.
  ///
  /// # Panics
  /// Panics when `prefix` exceeds 128.
  pub fn ipv6_prefix(mut self, prefix: u8) -> Self {
    assert!(prefix <= 128, "IPv6 prefix must be at most 128");
    self.cfg.ipv6_prefix = prefix;
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
  store: Arc<SccHashMap<BucketKey, Mutex<Bucket>>>,
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

    if !self.task_started.swap(true, Ordering::SeqCst) {
      let store = self.store.clone();

      let purge_after = idle_retention(&self.cfg);
      let interval = (purge_after / 2).min(Duration::from_secs(300));

      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
          tick.tick().await;
          evict_stale(&store, purge_after).await;
        }
      });

      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        loop {
          compio::time::sleep(interval).await;
          evict_stale(&store, purge_after).await;
        }
      })
      .detach();
    }

    Ok(())
  }
}

fn idle_retention(cfg: &Config) -> Duration {
  // Evicting before the full burst can refill would reset an exhausted quota too early.
  Duration::from_millis(cfg.refill_interval_ms)
    .saturating_mul(cfg.max_requests.div_ceil(cfg.refill_rate))
    .max(Duration::from_secs(300))
}

async fn evict_stale(store: &SccHashMap<BucketKey, Mutex<Bucket>>, purge_after: Duration) {
  let now = Instant::now();
  store
    .retain_async(|_, mutex| now.saturating_duration_since(mutex.lock().last_refill) < purge_after)
    .await;
}
