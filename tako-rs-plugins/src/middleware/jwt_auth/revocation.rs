//! Token revocation list and remote introspection hooks.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use scc::HashSet as SccHashSet;

/// Token revocation list interface (sync because revocation is on the hot
/// path and remote checks should go through a cache).
pub trait RevocationList: Send + Sync + 'static {
  fn is_revoked(&self, jti: &str) -> bool;
}

/// Default in-memory revocation list keyed by `jti` (JWT ID claim).
#[derive(Default, Clone)]
pub struct InMemoryRevocationList {
  inner: Arc<SccHashSet<String>>,
}

impl InMemoryRevocationList {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn revoke(&self, jti: impl Into<String>) {
    let _ = self.inner.insert_sync(jti.into());
  }

  pub fn unrevoke(&self, jti: &str) {
    let _ = self.inner.remove_sync(jti);
  }
}

impl RevocationList for InMemoryRevocationList {
  fn is_revoked(&self, jti: &str) -> bool {
    self.inner.contains_sync(jti)
  }
}

/// Optional remote introspection. Returns true when the token is still
/// valid; false when it has been revoked / expired upstream.
pub type IntrospectionFn =
  Arc<dyn Fn(&str) -> Pin<Box<dyn Future<Output = bool> + Send + 'static>> + Send + Sync + 'static>;

/// Closure that extracts a `jti` (or any revocation-list key) from the
/// verifier's decoded claims. Required when wiring up [`JwtAuth::revocation`](super::JwtAuth::revocation).
pub type JtiExtractorFn<C> = Arc<dyn Fn(&C) -> Option<String> + Send + Sync + 'static>;

/// Pair of [`RevocationList`] and a JTI extractor used to wire revocation onto a verifier.
pub type RevocationCheck<C> = (Arc<dyn RevocationList>, JtiExtractorFn<C>);
