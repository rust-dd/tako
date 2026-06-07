//! Rate-limiter configuration: algorithm choice, unkeyed-request policy,
//! runtime settings, and the custom key-function type.

use std::sync::Arc;

use http::StatusCode;
use tako_rs_core::types::Request;

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
  /// Tokens added per refill interval (`TokenBucket` only).
  pub refill_rate: u32,
  /// Refill interval (`TokenBucket` only).
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
