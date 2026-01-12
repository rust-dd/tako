#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Rate limiting plugin using token bucket algorithm for controlling request frequency.
//!
//! This module provides rate limiting functionality to protect Tako applications from abuse
//! and ensure fair resource usage. It implements a token bucket algorithm with per-IP tracking,
//! configurable burst sizes, and automatic token replenishment. The plugin maintains state
//! using a concurrent hash map and spawns a background task for token replenishment and
//! cleanup of inactive buckets.
//!
//! The rate limiter plugin can be applied at both router-level (all routes) and route-level
//! (specific routes), allowing different rate limits for different endpoints.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::rate_limiter::{RateLimiterPlugin, RateLimiterBuilder};
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use tako::Method;
//! use http::StatusCode;
//!
//! async fn handler(_req: tako::types::Request) -> &'static str {
//!     "Response"
//! }
//!
//! async fn api_handler(_req: tako::types::Request) -> &'static str {
//!     "API response"
//! }
//!
//! let mut router = Router::new();
//!
//! // Router-level: Basic rate limiting (50 req/sec, burst 100)
//! let global_limiter = RateLimiterBuilder::new()
//!     .max_requests(100)
//!     .refill_rate(50)
//!     .refill_interval_ms(1000)
//!     .build();
//! router.plugin(global_limiter);
//!
//! // Route-level: Stricter rate limiting for API (5 req/sec, burst 10)
//! let api_route = router.route(Method::POST, "/api/sensitive", api_handler);
//! let api_limiter = RateLimiterBuilder::new()
//!     .max_requests(10)
//!     .refill_rate(5)
//!     .refill_interval_ms(1000)
//!     .status(StatusCode::TOO_MANY_REQUESTS)
//!     .build();
//! api_route.plugin(api_limiter);
//! ```

use std::net::IpAddr;
use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use anyhow::Result;
use http::StatusCode;
use scc::HashMap as SccHashMap;

use crate::body::TakoBody;
use crate::middleware::Next;
use crate::plugins::TakoPlugin;
use crate::responder::Responder;
use crate::router::Router;
use crate::types::Request;

/// Rate limiter configuration parameters.
///
/// `Config` defines the behavior of the rate limiter including the maximum
/// number of requests allowed (capacity), request quota replenishment rate,
/// update frequency, and HTTP status code for rate limit violations. The rate limiter
/// allows for burst traffic up to the max capacity while maintaining an average rate over time.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::rate_limiter::Config;
/// use http::StatusCode;
///
/// // Allow 100 requests per second with burst up to 200
/// let config = Config {
///     max_requests: 200,
///     refill_rate: 100,
///     refill_interval_ms: 1000,
///     status_on_limit: StatusCode::TOO_MANY_REQUESTS,
/// };
/// ```
#[derive(Clone)]
pub struct Config {
  /// Maximum number of requests that can be made (capacity/burst limit).
  pub max_requests: u32,
  /// Number of requests to allow per refill interval.
  pub refill_rate: u32,
  /// Interval in milliseconds at which request quota is refilled.
  pub refill_interval_ms: u64,
  /// HTTP status code returned when the rate limit is exceeded.
  pub status_on_limit: StatusCode,
}

impl Default for Config {
  /// Provides sensible default rate limiting configuration: 60 requests per second.
  fn default() -> Self {
    Self {
      max_requests: 60,
      refill_rate: 60,
      refill_interval_ms: 1000,
      status_on_limit: StatusCode::TOO_MANY_REQUESTS,
    }
  }
}

/// Builder for configuring rate limiter settings with a fluent API.
///
/// `RateLimiterBuilder` provides a convenient way to construct rate limiter configurations
/// using method chaining. The rate limiter works by maintaining a quota of available requests where:
/// - `max_requests`: Maximum burst capacity (how many requests can be made at once)
/// - `refill_rate`: Number of requests allowed per refill interval
/// - `refill_interval_ms`: How often to refill the request quota (in milliseconds)
///
/// # Examples
///
/// ```rust
/// use tako::plugins::rate_limiter::RateLimiterBuilder;
/// use http::StatusCode;
///
/// // Allow 100 requests per second with burst up to 1000
/// let high_traffic = RateLimiterBuilder::new()
///     .max_requests(1000)
///     .refill_rate(100)
///     .refill_interval_ms(1000)
///     .build();
///
/// // Allow 5 requests per second, max 10 burst
/// let conservative = RateLimiterBuilder::new()
///     .max_requests(10)
///     .refill_rate(5)
///     .refill_interval_ms(1000)
///     .build();
///
/// // Allow 1 request per 500ms (2 per second)
/// let strict = RateLimiterBuilder::new()
///     .max_requests(1)
///     .refill_rate(1)
///     .refill_interval_ms(500)
///     .build();
/// ```
pub struct RateLimiterBuilder(Config);

impl RateLimiterBuilder {
  /// Creates a new rate limiter configuration builder with default settings.
  pub fn new() -> Self {
    Self(Config::default())
  }

  /// Sets the maximum number of requests allowed (capacity/burst limit).
  ///
  /// This is the maximum burst size - how many requests can be made at once.
  pub fn max_requests(mut self, n: u32) -> Self {
    self.0.max_requests = n;
    self
  }

  /// Sets how many requests to allow per refill interval.
  ///
  /// For example, `refill_rate(100)` with `refill_interval_ms(1000)` = 100 requests/second.
  pub fn refill_rate(mut self, n: u32) -> Self {
    self.0.refill_rate = n;
    self
  }

  /// Sets the refill interval in milliseconds.
  ///
  /// Request quota is replenished at this interval. For example:
  /// - `1000` = refill every second
  /// - `500` = refill every 500ms (twice per second)
  /// - `100` = refill every 100ms (10 times per second)
  pub fn refill_interval_ms(mut self, ms: u64) -> Self {
    self.0.refill_interval_ms = ms.max(1);
    self
  }

  /// Sets the HTTP status code returned when rate limits are exceeded.
  pub fn status(mut self, st: StatusCode) -> Self {
    self.0.status_on_limit = st;
    self
  }

  /// Builds the rate limiter plugin with the configured settings.
  pub fn build(self) -> RateLimiterPlugin {
    RateLimiterPlugin {
      cfg: self.0,
      store: Arc::new(SccHashMap::new()),
      task_started: Arc::new(AtomicBool::new(false)),
    }
  }

  /// Convenience method: Allow N requests per second with the same burst limit.
  ///
  /// This is a shorthand for setting max_requests, refill_rate, and refill_interval_ms
  /// to create a simple "N requests per second" limit.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::plugins::rate_limiter::RateLimiterBuilder;
  ///
  /// // Allow 100 requests per second
  /// let limiter = RateLimiterBuilder::new()
  ///     .requests_per_second(100)
  ///     .build();
  /// ```
  pub fn requests_per_second(mut self, n: u32) -> Self {
    self.0.max_requests = n;
    self.0.refill_rate = n;
    self.0.refill_interval_ms = 1000;
    self
  }

  /// Convenience method: Allow N requests per minute with the same burst limit.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::plugins::rate_limiter::RateLimiterBuilder;
  ///
  /// // Allow 600 requests per minute (10 per second)
  /// let limiter = RateLimiterBuilder::new()
  ///     .requests_per_minute(600)
  ///     .build();
  /// ```
  pub fn requests_per_minute(mut self, n: u32) -> Self {
    self.0.max_requests = n;
    self.0.refill_rate = n;
    self.0.refill_interval_ms = 60000;
    self
  }
}

/// Request quota tracker for rate limiting per IP address.
///
/// `Bucket` represents the state of request quota for a single IP address including
/// the current number of available requests and last access time. Each IP address
/// gets its own bucket for tracking rate limits independently.
///
/// # Examples
///
/// ```rust
/// use std::time::Instant;
///
/// # struct Bucket {
/// #     available: f64,
/// #     last_seen: Instant,
/// # }
/// let bucket = Bucket {
///     available: 60.0,
///     last_seen: Instant::now(),
/// };
/// ```
#[derive(Clone)]
struct Bucket {
  /// Current number of available requests remaining.
  available: f64,
  /// Last time this bucket was accessed for cleanup purposes.
  last_seen: Instant,
}

/// Rate limiting plugin with per-IP request quota tracking.
///
/// `RateLimiterPlugin` provides comprehensive rate limiting functionality by tracking
/// request quotas per IP address. It maintains per-IP state in a concurrent hash map,
/// spawns a background task for quota replenishment and cleanup, and integrates with
/// Tako's middleware system to enforce rate limits on incoming requests.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::rate_limiter::{RateLimiterPlugin, RateLimiterBuilder};
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
///
/// // Create and configure rate limiter: 50 requests/sec, max 100 burst
/// let limiter = RateLimiterBuilder::new()
///     .max_requests(100)
///     .refill_rate(50)
///     .refill_interval_ms(1000)
///     .build();
///
/// // Apply to router
/// let mut router = Router::new();
/// router.plugin(limiter);
/// ```
#[derive(Clone)]
#[doc(alias = "rate_limiter")]
#[doc(alias = "ratelimit")]
pub struct RateLimiterPlugin {
  /// Rate limiting configuration parameters.
  cfg: Config,
  /// Concurrent map storing token buckets for each IP address.
  store: Arc<SccHashMap<IpAddr, Bucket>>,
  /// Flag to ensure background task is spawned only once.
  task_started: Arc<AtomicBool>,
}

impl TakoPlugin for RateLimiterPlugin {
  /// Returns the plugin name for identification and debugging.
  fn name(&self) -> &'static str {
    "RateLimiterPlugin"
  }

  /// Sets up the rate limiter by registering middleware and starting background tasks.
  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = self.cfg.clone();
    let store = self.store.clone();

    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      let store = store.clone();
      async move { retain(req, next, cfg, store).await }
    });

    // Only spawn the background task once per plugin instance
    if !self.task_started.swap(true, Ordering::SeqCst) {
      let cfg = self.cfg.clone();
      let store = self.store.clone();

      #[cfg(not(feature = "compio"))]
      tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_millis(cfg.refill_interval_ms));
        let requests_to_add = cfg.refill_rate as f64;
        let purge_after = Duration::from_secs(300);
        loop {
          tick.tick().await;
          let now = Instant::now();
          store.retain_sync(|_, b| {
            b.available = (b.available + requests_to_add).min(cfg.max_requests as f64);
            now.duration_since(b.last_seen) < purge_after
          });
        }
      });

      #[cfg(feature = "compio")]
      compio::runtime::spawn(async move {
        let requests_to_add = cfg.refill_rate as f64;
        let purge_after = Duration::from_secs(300);
        let interval = Duration::from_millis(cfg.refill_interval_ms);
        loop {
          compio::time::sleep(interval).await;
          let now = Instant::now();
          store.retain(|_, b| {
            b.available = (b.available + requests_to_add).min(cfg.max_requests as f64);
            now.duration_since(b.last_seen) < purge_after
          });
        }
      })
      .detach();
    }

    Ok(())
  }
}

/// Middleware function that enforces rate limiting per IP address.
///
/// This function extracts the client IP address from the request, checks if they have
/// available request quota remaining, and either allows the request to proceed or
/// returns a rate limit error response. It updates quota state atomically and handles
/// new clients by creating buckets with full quota.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::plugins::rate_limiter::{retain, Config};
/// use tako::middleware::Next;
/// use tako::types::Request;
/// use std::sync::Arc;
/// use scc::HashMap as SccHashMap;
///
/// # async fn example() {
/// # let req = Request::builder().body(tako::body::TakoBody::empty()).unwrap();
/// # let next = Next { middlewares: Arc::new(vec![]), endpoint: Arc::new(|_| Box::pin(async { tako::types::Response::new(tako::body::TakoBody::empty()) })) };
/// let config = Config::default();
/// let store = Arc::new(SccHashMap::new());
/// let response = retain(req, next, config, store).await;
/// # }
/// ```
async fn retain(
  req: Request,
  next: Next,
  cfg: Config,
  store: Arc<SccHashMap<IpAddr, Bucket>>,
) -> impl Responder {
  let ip = req
    .extensions()
    .get::<SocketAddr>()
    .map(|sa| sa.ip())
    .unwrap_or(IpAddr::from([0, 0, 0, 0]));

  let mut entry = store.entry_sync(ip).or_insert_with(|| Bucket {
    available: cfg.max_requests as f64,
    last_seen: Instant::now(),
  });

  if entry.available < 1.0 {
    return http::Response::builder()
      .status(cfg.status_on_limit)
      .body(TakoBody::empty())
      .unwrap();
  }
  entry.available -= 1.0;
  entry.last_seen = Instant::now();
  drop(entry);

  next.run(req).await
}
