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
//! // Router-level: Basic rate limiting (applied to all routes)
//! let global_limiter = RateLimiterBuilder::new()
//!     .burst_size(100)
//!     .per_second(50)
//!     .build();
//! router.plugin(global_limiter);
//!
//! // Route-level: Stricter rate limiting for API endpoint
//! let api_route = router.route(Method::POST, "/api/sensitive", api_handler);
//! let api_limiter = RateLimiterBuilder::new()
//!     .burst_size(10)
//!     .per_second(5)
//!     .tick_secs(1)
//!     .status(StatusCode::TOO_MANY_REQUESTS)
//!     .build();
//! api_route.plugin(api_limiter);
//! ```

use std::{
    net::{IpAddr, SocketAddr},
    sync::{Arc, atomic::{AtomicBool, Ordering}},
    time::{Duration, Instant},
};

use anyhow::Result;
use dashmap::DashMap;
use http::StatusCode;
use tokio::time;

use crate::{
    body::TakoBody, middleware::Next, plugins::TakoPlugin, responder::Responder, router::Router,
    types::Request,
};

/// Rate limiter configuration using token bucket algorithm parameters.
///
/// `Config` defines the behavior of the token bucket rate limiter including the maximum
/// burst capacity, token replenishment rate, update frequency, and HTTP status code for
/// rate limit violations. The token bucket allows for burst traffic up to the bucket
/// capacity while maintaining an average rate over time.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::rate_limiter::Config;
/// use http::StatusCode;
///
/// let config = Config {
///     burst_size: 200,
///     per_second: 100,
///     tick_secs: 1,
///     status_on_limit: StatusCode::TOO_MANY_REQUESTS,
/// };
/// ```
#[derive(Clone)]
pub struct Config {
    /// Maximum number of tokens (requests) that can be accumulated in the bucket.
    pub burst_size: u32,
    /// Rate at which tokens are added to the bucket per second.
    pub per_second: u32,
    /// Interval in seconds at which tokens are replenished.
    pub tick_secs: u64,
    /// HTTP status code returned when the rate limit is exceeded.
    pub status_on_limit: StatusCode,
}

impl Default for Config {
    /// Provides sensible default rate limiting configuration.
    fn default() -> Self {
        Self {
            burst_size: 60,
            per_second: 60,
            tick_secs: 1,
            status_on_limit: StatusCode::TOO_MANY_REQUESTS,
        }
    }
}

/// Builder for configuring rate limiter settings with a fluent API.
///
/// `RateLimiterBuilder` provides a convenient way to construct rate limiter configurations
/// using method chaining. It starts with sensible defaults and allows customization of
/// burst capacity, rate limits, timing intervals, and response status codes for rate
/// limit violations.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::rate_limiter::RateLimiterBuilder;
/// use http::StatusCode;
///
/// // High-traffic API configuration
/// let api_limiter = RateLimiterBuilder::new()
///     .burst_size(1000)
///     .per_second(500)
///     .tick_secs(1)
///     .status(StatusCode::TOO_MANY_REQUESTS)
///     .build();
///
/// // Conservative rate limiting
/// let conservative = RateLimiterBuilder::new()
///     .burst_size(10)
///     .per_second(5)
///     .build();
/// ```
pub struct RateLimiterBuilder(Config);

impl RateLimiterBuilder {
    /// Creates a new rate limiter configuration builder with default settings.
    pub fn new() -> Self {
        Self(Config::default())
    }

    /// Sets the maximum burst size for the token bucket.
    pub fn burst_size(mut self, n: u32) -> Self {
        self.0.burst_size = n;
        self
    }

    /// Sets the token replenishment rate per second.
    pub fn per_second(mut self, n: u32) -> Self {
        self.0.per_second = n;
        self
    }

    /// Sets the token replenishment interval in seconds.
    pub fn tick_secs(mut self, s: u64) -> Self {
        self.0.tick_secs = s.max(1);
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
            store: Arc::new(DashMap::new()),
            task_started: Arc::new(AtomicBool::new(false)),
        }
    }
}

/// Token bucket for tracking request allowance per IP address.
///
/// `Bucket` represents the state of a single token bucket including the current
/// token count and last access time. The bucket is used to implement the token
/// bucket algorithm for rate limiting individual IP addresses.
///
/// # Examples
///
/// ```rust
/// use std::time::Instant;
///
/// # struct Bucket {
/// #     tokens: f64,
/// #     last_seen: Instant,
/// # }
/// let bucket = Bucket {
///     tokens: 60.0,
///     last_seen: Instant::now(),
/// };
/// ```
#[derive(Clone)]
struct Bucket {
    /// Current number of tokens available for requests.
    tokens: f64,
    /// Last time this bucket was accessed for cleanup purposes.
    last_seen: Instant,
}

/// Rate limiting plugin implementing token bucket algorithm with per-IP tracking.
///
/// `RateLimiterPlugin` provides comprehensive rate limiting functionality using a token
/// bucket algorithm. It maintains per-IP state in a concurrent hash map, spawns a
/// background task for token replenishment and cleanup, and integrates with Tako's
/// middleware system to enforce rate limits on incoming requests.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::rate_limiter::{RateLimiterPlugin, RateLimiterBuilder};
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
///
/// // Create and configure rate limiter
/// let limiter = RateLimiterBuilder::new()
///     .burst_size(100)
///     .per_second(50)
///     .build();
///
/// // Apply to router
/// let mut router = Router::new();
/// router.plugin(limiter);
/// ```
#[derive(Clone)]
pub struct RateLimiterPlugin {
    /// Rate limiting configuration parameters.
    cfg: Config,
    /// Concurrent map storing token buckets for each IP address.
    store: Arc<DashMap<IpAddr, Bucket>>,
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

            tokio::spawn(async move {
                let mut tick = time::interval(Duration::from_secs(cfg.tick_secs));
                let add_per_tick = cfg.per_second as f64 * cfg.tick_secs as f64;
                let purge_after = Duration::from_secs(300);
                loop {
                    tick.tick().await;
                    let now = Instant::now();
                    store.retain(|_, b| {
                        b.tokens = (b.tokens + add_per_tick).min(cfg.burst_size as f64);
                        now.duration_since(b.last_seen) < purge_after
                    });
                }
            });
        }

        Ok(())
    }
}

/// Middleware function that enforces rate limiting using token bucket algorithm.
///
/// This function extracts the client IP address from the request, checks if sufficient
/// tokens are available in their bucket, and either allows the request to proceed or
/// returns a rate limit error response. It updates bucket state atomically and handles
/// new clients by creating buckets with full token capacity.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::plugins::rate_limiter::{retain, Config};
/// use tako::middleware::Next;
/// use tako::types::Request;
/// use std::sync::Arc;
/// use dashmap::DashMap;
///
/// # async fn example() {
/// # let req = Request::builder().body(tako::body::TakoBody::empty()).unwrap();
/// # let next = Next { middlewares: Arc::new(vec![]), endpoint: Arc::new(|_| Box::pin(async { tako::types::Response::new(tako::body::TakoBody::empty()) })) };
/// let config = Config::default();
/// let store = Arc::new(DashMap::new());
/// let response = retain(req, next, config, store).await;
/// # }
/// ```
async fn retain(
    req: Request,
    next: Next,
    cfg: Config,
    store: Arc<DashMap<IpAddr, Bucket>>,
) -> impl Responder {
    let ip = req
        .extensions()
        .get::<SocketAddr>()
        .map(|sa| sa.ip())
        .unwrap_or(IpAddr::from([0, 0, 0, 0]));

    let mut entry = store.entry(ip).or_insert_with(|| Bucket {
        tokens: cfg.burst_size as f64,
        last_seen: Instant::now(),
    });

    if entry.tokens < 1.0 {
        return hyper::Response::builder()
            .status(cfg.status_on_limit)
            .body(TakoBody::empty())
            .unwrap();
    }
    entry.tokens -= 1.0;
    entry.last_seen = Instant::now();
    drop(entry);

    next.run(req).await
}
