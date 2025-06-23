/// The `RateLimiterPlugin` provides rate-limiting functionality for the Tako framework.
/// It uses a token bucket algorithm to control the rate of requests from individual IP addresses.
///
/// # Configuration
/// The plugin can be configured using the `RateLimiterBuilder`:
/// - `burst_size`: Maximum number of tokens (requests) that can be accumulated in the bucket.
/// - `per_second`: Rate at which tokens are added to the bucket per second.
/// - `tick_secs`: Interval (in seconds) at which tokens are replenished.
/// - `status_on_limit`: HTTP status code returned when the rate limit is exceeded.
///
/// # Example
/// ```rust
/// use tako::plugins::rate_limiter::RateLimiterBuilder;
///
/// let rate_limiter = RateLimiterBuilder::new()
///     .burst_size(100)
///     .per_second(50)
///     .status(http::StatusCode::TOO_MANY_REQUESTS)
///     .build();
///
/// router.plugin(rate_limiter);
/// ```
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

/// Configuration for the `RateLimiterPlugin`.
///
/// - `burst_size`: Maximum number of tokens (requests) that can be accumulated in the bucket.
/// - `per_second`: Rate at which tokens are added to the bucket per second.
/// - `tick_secs`: Interval (in seconds) at which tokens are replenished.
/// - `status_on_limit`: HTTP status code returned when the rate limit is exceeded.
#[derive(Clone)]
pub struct Config {
    pub burst_size: u32,
    pub per_second: u32,
    pub tick_secs: u64,
    pub status_on_limit: StatusCode,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            burst_size: 60,
            per_second: 60,
            tick_secs: 1,
            status_on_limit: StatusCode::TOO_MANY_REQUESTS,
        }
    }
}

/// Builder for the `RateLimiterPlugin`.
///
/// Provides a fluent API to configure and create an instance of the rate limiter.
pub struct RateLimiterBuilder(Config);

impl RateLimiterBuilder {
    pub fn new() -> Self {
        Self(Config::default())
    }

    pub fn burst_size(mut self, n: u32) -> Self {
        self.0.burst_size = n;
        self
    }

    pub fn per_second(mut self, n: u32) -> Self {
        self.0.per_second = n;
        self
    }

    pub fn tick_secs(mut self, s: u64) -> Self {
        self.0.tick_secs = s.max(1);
        self
    }

    pub fn status(mut self, st: StatusCode) -> Self {
        self.0.status_on_limit = st;
        self
    }

    pub fn build(self) -> RateLimiterPlugin {
        RateLimiterPlugin {
            cfg: self.0,
            store: Arc::new(DashMap::new()),
        }
    }
}

/// Represents a token bucket for rate limiting.
///
/// - `tokens`: The current number of tokens available.
/// - `last_seen`: The last time the bucket was accessed.
/// The `RateLimiterPlugin` implements the `TakoPlugin` trait and provides rate-limiting functionality.
///
/// It maintains a `DashMap` to store token buckets for each IP address and enforces rate limits
/// based on the configured parameters.
#[derive(Clone)]
struct Bucket {
    tokens: f64,
    last_seen: Instant,
}

#[derive(Clone)]
pub struct RateLimiterPlugin {
    cfg: Config,
    store: Arc<DashMap<IpAddr, Bucket>>,
}

impl TakoPlugin for RateLimiterPlugin {
    /// Returns the name of the plugin: `"RateLimiterPlugin"`.
    fn name(&self) -> &'static str {
        "RateLimiterPlugin"
    }

    /// Sets up the rate limiter by attaching middleware to the router and spawning a background task
    /// to replenish tokens and purge inactive buckets.
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

/// Middleware function to enforce rate limiting.
///
/// Checks if the IP address has sufficient tokens to process the request.
/// If tokens are available, the request is allowed; otherwise, it is rejected with the configured status code.
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
