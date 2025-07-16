//! Rate limiting plugin using token bucket algorithm for controlling request frequency.
//!
//! This module provides rate limiting functionality to protect Tako applications from abuse
//! and ensure fair resource usage. It implements a token bucket algorithm with per-IP tracking,
//! configurable burst sizes, and automatic token replenishment. The plugin maintains state
//! using a concurrent hash map and spawns a background task for token replenishment and
//! cleanup of inactive buckets.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::rate_limiter::{RateLimiterPlugin, RateLimiterBuilder};
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use http::StatusCode;
//!
//! // Basic rate limiting setup
//! let rate_limiter = RateLimiterBuilder::new()
//!     .burst_size(100)
//!     .per_second(50)
//!     .build();
//!
//! let mut router = Router::new();
//! router.plugin(rate_limiter);
//!
//! // Custom rate limiting for API endpoints
//! let api_limiter = RateLimiterBuilder::new()
//!     .burst_size(1000)
//!     .per_second(100)
//!     .tick_secs(1)
//!     .status(StatusCode::TOO_MANY_REQUESTS)
//!     .build();
//! router.plugin(api_limiter);
//! ```

use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
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
    ///
    /// Default settings allow 60 requests per second with a burst capacity of 60 requests,
    /// token replenishment every second, and returns HTTP 429 (Too Many Requests) status
    /// when limits are exceeded.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::Config;
    /// use http::StatusCode;
    ///
    /// let config = Config::default();
    /// assert_eq!(config.burst_size, 60);
    /// assert_eq!(config.per_second, 60);
    /// assert_eq!(config.tick_secs, 1);
    /// assert_eq!(config.status_on_limit, StatusCode::TOO_MANY_REQUESTS);
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterBuilder;
    ///
    /// let builder = RateLimiterBuilder::new();
    /// let limiter = builder.build();
    /// ```
    pub fn new() -> Self {
        Self(Config::default())
    }

    /// Sets the maximum burst size for the token bucket.
    ///
    /// The burst size determines how many requests can be made in quick succession
    /// before rate limiting takes effect. Higher values allow for more bursty
    /// traffic patterns while maintaining the overall rate limit.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterBuilder;
    ///
    /// let limiter = RateLimiterBuilder::new()
    ///     .burst_size(500) // Allow up to 500 requests in burst
    ///     .build();
    /// ```
    pub fn burst_size(mut self, n: u32) -> Self {
        self.0.burst_size = n;
        self
    }

    /// Sets the token replenishment rate per second.
    ///
    /// This determines the sustained rate at which requests are allowed over time.
    /// The bucket is replenished with this many tokens every second (distributed
    /// across tick intervals).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterBuilder;
    ///
    /// let limiter = RateLimiterBuilder::new()
    ///     .per_second(100) // Allow 100 requests per second sustained
    ///     .build();
    /// ```
    pub fn per_second(mut self, n: u32) -> Self {
        self.0.per_second = n;
        self
    }

    /// Sets the token replenishment interval in seconds.
    ///
    /// This controls how frequently tokens are added to buckets. Shorter intervals
    /// provide smoother rate limiting but higher overhead. The minimum value is 1 second.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterBuilder;
    ///
    /// let limiter = RateLimiterBuilder::new()
    ///     .tick_secs(2) // Replenish tokens every 2 seconds
    ///     .build();
    /// ```
    pub fn tick_secs(mut self, s: u64) -> Self {
        self.0.tick_secs = s.max(1);
        self
    }

    /// Sets the HTTP status code returned when rate limits are exceeded.
    ///
    /// This allows customization of the error response when clients exceed their
    /// rate limits. Common values include 429 (Too Many Requests) or 503 (Service
    /// Unavailable).
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterBuilder;
    /// use http::StatusCode;
    ///
    /// let limiter = RateLimiterBuilder::new()
    ///     .status(StatusCode::SERVICE_UNAVAILABLE)
    ///     .build();
    /// ```
    pub fn status(mut self, st: StatusCode) -> Self {
        self.0.status_on_limit = st;
        self
    }

    /// Builds the rate limiter plugin with the configured settings.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterBuilder;
    /// use tako::plugins::TakoPlugin;
    /// use tako::router::Router;
    ///
    /// let limiter = RateLimiterBuilder::new()
    ///     .burst_size(100)
    ///     .per_second(50)
    ///     .build();
    ///
    /// let mut router = Router::new();
    /// router.plugin(limiter);
    /// ```
    pub fn build(self) -> RateLimiterPlugin {
        RateLimiterPlugin {
            cfg: self.0,
            store: Arc::new(DashMap::new()),
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
}

impl TakoPlugin for RateLimiterPlugin {
    /// Returns the plugin name for identification and debugging.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::RateLimiterPlugin;
    /// use tako::plugins::TakoPlugin;
    ///
    /// let plugin = RateLimiterPlugin {
    ///     cfg: Default::default(),
    ///     store: Default::default(),
    /// };
    /// assert_eq!(plugin.name(), "RateLimiterPlugin");
    /// ```
    fn name(&self) -> &'static str {
        "RateLimiterPlugin"
    }

    /// Sets up the rate limiter by registering middleware and starting background tasks.
    ///
    /// This method installs the rate limiting middleware that checks and updates token
    /// buckets for each request. It also spawns a background task that periodically
    /// replenishes tokens and cleans up inactive buckets to prevent memory leaks.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::rate_limiter::{RateLimiterPlugin, RateLimiterBuilder};
    /// use tako::plugins::TakoPlugin;
    /// use tako::router::Router;
    ///
    /// let plugin = RateLimiterBuilder::new().build();
    /// let router = Router::new();
    /// plugin.setup(&router).unwrap();
    /// ```
    fn setup(&self, router: &Router) -> Result<()> {
        let cfg = self.cfg.clone();
        let store = self.store.clone();

        router.middleware(move |req, next| {
            let cfg = cfg.clone();
            let store = store.clone();
            async move { retain(req, next, cfg, store).await }
        });

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
