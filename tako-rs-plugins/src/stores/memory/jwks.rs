use std::sync::Arc;

use async_trait::async_trait;
use scc::HashMap as SccHashMap;

use crate::stores::JwksProvider;
use crate::stores::StoreResult;
use crate::stores::VerificationKey;
/// Static-snapshot JWKS provider.
#[derive(Default, Clone)]
pub struct StaticJwksProvider {
  by_kid: Arc<SccHashMap<String, Vec<VerificationKey>>>,
}

impl StaticJwksProvider {
  pub fn new() -> Self {
    Self::default()
  }

  /// Adds a key under `kid`. Multiple keys per kid are supported (rotation).
  pub fn insert(&self, kid: impl Into<String>, key: VerificationKey) {
    let kid = kid.into();
    self
      .by_kid
      .entry_sync(kid)
      .and_modify(|v| v.push(key.clone()))
      .or_insert_with(|| vec![key]);
  }
}

#[async_trait]
impl JwksProvider for StaticJwksProvider {
  async fn keys_for(&self, kid: &str) -> StoreResult<Vec<VerificationKey>> {
    Ok(
      self
        .by_kid
        .get_async(kid)
        .await
        .map(|v| v.clone())
        .unwrap_or_default(),
    )
  }
}
