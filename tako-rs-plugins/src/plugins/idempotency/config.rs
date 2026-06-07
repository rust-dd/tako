//! Idempotency cache policy, matching configuration, and the builder.

use http::HeaderName;
use http::Method;

use super::plugin::IdempotencyPlugin;

/// Which request attributes are included in the idempotency key scope.
#[derive(Clone, Copy)]
pub enum Scope {
  /// Only the header value identifies the operation.
  KeyOnly,
  /// Header value combined with HTTP method and path.
  MethodAndPath,
}

/// Cache policy and matching configuration.
#[derive(Clone)]
pub struct Config {
  /// Header that carries the idempotency key.
  pub header: HeaderName,
  /// Methods to protect. Default: `[POST]`.
  pub methods: Vec<Method>,
  /// Time-to-live for cached results (seconds). Default: 86400 (24h).
  pub ttl_secs: u64,
  /// Include method+path in the cache key. Default: `MethodAndPath`.
  pub scope: Scope,
  /// If true, concurrent calls with same key wait for the first to finish. Default: true.
  pub coalesce_inflight: bool,
  /// Optional timeout for waiting on in-flight (milliseconds). Default: None (wait indefinitely).
  pub inflight_wait_timeout_ms: Option<u64>,
  /// Maximum response body size to cache (bytes). Default: 1 MiB.
  pub max_cached_body_bytes: usize,
  /// Maximum request body size to hash (bytes). Requests exceeding this are rejected with 413.
  pub max_request_body_bytes: usize,
  /// If true, enforce identical payload for the same key; otherwise only the key is checked.
  pub verify_payload: bool,
  /// If true, also cache non-success statuses. Default: true.
  pub cache_error_statuses: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      header: HeaderName::from_static("idempotency-key"),
      methods: vec![Method::POST],
      // Matches the documented default on `Config::ttl_secs` (24h).
      ttl_secs: 86400,
      scope: Scope::MethodAndPath,
      coalesce_inflight: true,
      inflight_wait_timeout_ms: None,
      max_cached_body_bytes: 1024 * 1024,
      max_request_body_bytes: 1024 * 1024,
      verify_payload: true,
      cache_error_statuses: true,
    }
  }
}

/// Builder for the idempotency plugin.
pub struct IdempotencyBuilder(Config);

impl Default for IdempotencyBuilder {
  fn default() -> Self {
    Self::new()
  }
}

impl IdempotencyBuilder {
  /// Start with sensible defaults.
  pub fn new() -> Self {
    Self(Config::default())
  }
  pub fn header(mut self, h: HeaderName) -> Self {
    self.0.header = h;
    self
  }
  pub fn methods(mut self, m: &[Method]) -> Self {
    self.0.methods = m.to_vec();
    self
  }
  pub fn ttl_secs(mut self, s: u64) -> Self {
    self.0.ttl_secs = s;
    self
  }
  pub fn scope(mut self, s: Scope) -> Self {
    self.0.scope = s;
    self
  }
  pub fn coalesce_inflight(mut self, yes: bool) -> Self {
    self.0.coalesce_inflight = yes;
    self
  }
  pub fn inflight_wait_timeout_ms(mut self, ms: Option<u64>) -> Self {
    self.0.inflight_wait_timeout_ms = ms;
    self
  }
  pub fn max_cached_body_bytes(mut self, n: usize) -> Self {
    self.0.max_cached_body_bytes = n;
    self
  }
  pub fn max_request_body_bytes(mut self, n: usize) -> Self {
    self.0.max_request_body_bytes = n;
    self
  }
  pub fn verify_payload(mut self, yes: bool) -> Self {
    self.0.verify_payload = yes;
    self
  }
  pub fn cache_error_statuses(mut self, yes: bool) -> Self {
    self.0.cache_error_statuses = yes;
    self
  }
  pub fn build(self) -> IdempotencyPlugin {
    IdempotencyPlugin::new(self.0)
  }
}
