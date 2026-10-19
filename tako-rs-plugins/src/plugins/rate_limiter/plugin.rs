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
use crate::stores::RateLimitStore;
use crate::stores::runtime;

#[cfg(test)]
mod tests;

/// Defaults: a token bucket of 60 requests refilled every second, keyed by peer IP.
pub struct RateLimiterBuilder {
  cfg: Config,
  key_fn: Option<KeyFn>,
  backend: Option<Arc<dyn RateLimitStore>>,
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
      backend: None,
    }
  }

  /// Use a shared backend, which owns its quota and expiration policy.
  /// Local capacity/refill/algorithm settings apply only to the default memory limiter.
  pub fn store(mut self, store: impl RateLimitStore) -> Self {
    self.backend = Some(Arc::new(store));
    self
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

  /// Groups IPv6 clients by prefix: 64 per subnet, 128 (the default) per address. Panics
  /// above 128.
  pub fn ipv6_prefix(mut self, prefix: u8) -> Self {
    assert!(prefix <= 128, "IPv6 prefix must be at most 128");
    self.cfg.ipv6_prefix = prefix;
    self
  }

  /// Replaces the peer-IP key, e.g. per route and IP or per tenant header; `None` follows
  /// [`Self::on_unkeyed`].
  pub fn key_fn<F>(mut self, f: F) -> Self
  where
    F: Fn(&Request) -> Option<String> + Send + Sync + 'static,
  {
    self.key_fn = Some(Arc::new(f));
    self
  }

  /// Burst and refill of `n` per second.
  pub fn requests_per_second(mut self, n: u32) -> Self {
    self.cfg.max_requests = n;
    self.cfg.refill_rate = n;
    self.cfg.refill_interval_ms = 1_000;
    self
  }

  /// Burst and refill of `n` per minute.
  pub fn requests_per_minute(mut self, n: u32) -> Self {
    self.cfg.max_requests = n;
    self.cfg.refill_rate = n;
    self.cfg.refill_interval_ms = 60_000;
    self
  }

  /// Panics on a zero `refill_rate`, `refill_interval_ms` or `max_requests`: a zero rate
  /// breaks GCRA's arithmetic and a zero cap silently denies every request.
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
      backend: self.backend,
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
  backend: Option<Arc<dyn RateLimitStore>>,
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
    let backend = self.backend.clone();

    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      let store = store.clone();
      let key_fn = key_fn.clone();
      let backend = backend.clone();
      async move { handle(req, next, cfg, store, key_fn, backend).await }
    });

    if !self.task_started.swap(true, Ordering::SeqCst) {
      if let Some(backend) = &self.backend {
        runtime::sweep(backend, Duration::from_secs(150), |backend| {
          Box::pin(async move { backend.sweep().await })
        });
        return Ok(());
      }
      let store = Arc::downgrade(&self.store);

      let purge_after = idle_retention(&self.cfg);
      let interval = (purge_after / 2).min(Duration::from_secs(300));

      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(interval);
        loop {
          tick.tick().await;
          let Some(store) = store.upgrade() else {
            break;
          };
          evict_stale(&store, purge_after).await;
        }
      });

      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        loop {
          compio::time::sleep(interval).await;
          let Some(store) = store.upgrade() else {
            break;
          };
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
