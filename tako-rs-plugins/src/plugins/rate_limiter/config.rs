use std::sync::Arc;

use http::StatusCode;
use tako_rs_core::types::Request;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algorithm {
  /// Refills continuously at `refill_rate` per `refill_interval_ms`, up to `max_requests`.
  TokenBucket,
  /// Generic Cell Rate Algorithm at the same rate and burst; its state is one time debt.
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

#[derive(Clone)]
pub struct Config {
  /// Burst capacity.
  pub max_requests: u32,
  /// With `refill_interval_ms`, sets the sustained rate of both algorithms.
  pub refill_rate: u32,
  pub refill_interval_ms: u64,
  /// Default: 429.
  pub status_on_limit: StatusCode,
  pub algorithm: Algorithm,
  /// Default: `Allow`.
  pub on_unkeyed: UnkeyedBehavior,
  /// Resolve trusted forwarding headers through the router's `IpAddrConfig`.
  pub client_ip: bool,
  /// IPv6 bucket prefix length. Defaults to 128; use 64 to group a subnet.
  pub ipv6_prefix: u8,
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
      client_ip: false,
      ipv6_prefix: 128,
    }
  }
}

/// Custom key function: maps a request to a rate-limit bucket id. Returning
/// `None` defers to [`Config::on_unkeyed`].
pub type KeyFn = Arc<dyn Fn(&Request) -> Option<String> + Send + Sync + 'static>;
