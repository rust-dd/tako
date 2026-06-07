#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Rate limiting plugin: token-bucket or GCRA, with composite keys and
//! IETF rate-limit response headers.
//!
//! v2 additions over the original token-bucket-by-IP design:
//!
//! - **Composite keys.** Default key is still the peer IP, but
//!   [`RateLimiterBuilder::key_fn`](crate::plugins::rate_limiter::RateLimiterBuilder::key_fn) lets callers compose per-route /
//!   per-tenant / per-user buckets without forking the plugin.
//! - **Strict IP fallback.** Requests without a discoverable peer IP no
//!   longer all collapse into the `0.0.0.0` bucket — the request is treated
//!   as unkeyed and skipped (configurable via [`RateLimiterBuilder::on_unkeyed`](crate::plugins::rate_limiter::RateLimiterBuilder::on_unkeyed)).
//! - **`RateLimit-*` headers.** Emits `RateLimit-Limit`, `RateLimit-Remaining`,
//!   `RateLimit-Reset`, and `Retry-After` per the IETF httpapi draft.
//! - **GCRA mode.** Opt in via [`Algorithm::Gcra`](crate::plugins::rate_limiter::Algorithm::Gcra). The per-key state stays
//!   one f64; no separate refill ticker.

mod algorithm;
mod config;
mod plugin;

pub use config::Algorithm;
pub use config::Config;
pub use config::KeyFn;
pub use config::UnkeyedBehavior;
pub use plugin::RateLimiterBuilder;
pub use plugin::RateLimiterPlugin;
