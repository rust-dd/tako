//! Token-bucket or GCRA rate limiting keyed by peer IP or a custom key, with IETF
//! `RateLimit-*` headers; requests without a key follow [`UnkeyedBehavior`].

mod algorithm;
mod config;
mod key;
mod plugin;

pub use config::Algorithm;
pub use config::Config;
pub use config::KeyFn;
pub use config::UnkeyedBehavior;
pub use plugin::RateLimiterBuilder;
pub use plugin::RateLimiterPlugin;
