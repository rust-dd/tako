use http::HeaderName;
use http::Method;
use tako_rs_core::types::Request;

use super::identity::IdempotencyIdentity;
use super::identity::IdentitySource;
use super::plugin::IdempotencyPlugin;

/// Request attributes added to the authenticated identity's idempotency scope.
#[derive(Clone, Copy)]
pub enum Scope {
  /// The header value identifies the operation within the identity scope.
  KeyOnly,
  /// Header value combined with HTTP method and path within the identity scope.
  MethodAndPath,
}

#[derive(Clone)]
pub struct Config {
  /// Default: `Idempotency-Key`.
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
  /// Maximum request body size to hash (bytes); larger requests get 413. Default: 1 MiB.
  pub max_request_body_bytes: usize,
  /// Reusing a key with another payload gets 409; otherwise only the key is compared.
  pub verify_payload: bool,
  /// If true, also cache non-success statuses. Default: true.
  pub cache_error_statuses: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      header: HeaderName::from_static("idempotency-key"),
      methods: vec![Method::POST],
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

/// Configure idempotency; the default requires a trusted `IdempotencyIdentity` extension.
pub struct IdempotencyBuilder(
  Config,
  Option<std::sync::Arc<dyn crate::stores::IdempotencyStore>>,
  IdentitySource,
);

impl Default for IdempotencyBuilder {
  fn default() -> Self {
    Self::new()
  }
}

impl IdempotencyBuilder {
  pub fn new() -> Self {
    Self(Config::default(), None, IdentitySource::default())
  }
  /// Extract trusted identity after authentication; missing or empty identities return 401.
  ///
  /// `None` never falls back to request headers, extensions, or anonymous scope.
  pub fn identity_fn(
    mut self,
    f: impl Fn(&Request) -> Option<IdempotencyIdentity> + Send + Sync + 'static,
  ) -> Self {
    self.2 = IdentitySource::Extractor(std::sync::Arc::new(f));
    self
  }
  /// Share keys across callers for public operations whose responses are safe to share.
  ///
  /// This replaces the identity extractor and intentionally ignores authenticated identity.
  pub fn anonymous(mut self) -> Self {
    self.2 = IdentitySource::Anonymous;
    self
  }
  /// Use a shared backend with atomic leases and completion.
  pub fn store(mut self, store: impl crate::stores::IdempotencyStore) -> Self {
    self.1 = Some(std::sync::Arc::new(store));
    self
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
    let mut plugin = IdempotencyPlugin::new(self.0);
    plugin.identity_source = self.2;
    if let Some(store) = self.1 {
      plugin.store = store;
    }
    plugin
  }
}
