use std::sync::Arc;

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use sha2::Digest;
use sha2::Sha256;
use tako_rs_core::types::Request;

use super::config::Scope;

/// Trusted identity set by authentication middleware before idempotency runs.
#[derive(Clone, Debug)]
pub struct IdempotencyIdentity {
  principal: String,
  tenant: Option<String>,
}

impl IdempotencyIdentity {
  /// Scope retries to a stable authenticated principal; empty identities are rejected.
  pub fn new(principal: impl Into<String>) -> Self {
    Self {
      principal: principal.into(),
      tenant: None,
    }
  }

  /// Further isolate the principal by an authenticated tenant; empty tenants are rejected.
  pub fn tenant(mut self, tenant: impl Into<String>) -> Self {
    self.tenant = Some(tenant.into());
    self
  }
}

type IdentityFn = dyn Fn(&Request) -> Option<IdempotencyIdentity> + Send + Sync;

#[derive(Clone, Default)]
pub(crate) enum IdentitySource {
  #[default]
  Extension,
  Extractor(Arc<IdentityFn>),
  Anonymous,
}

impl IdentitySource {
  pub(crate) fn cache_key(&self, request: &Request, scope: Scope, key: &str) -> Option<String> {
    let identity = match self {
      Self::Extension => Some(request.extensions().get::<IdempotencyIdentity>().cloned()?),
      Self::Extractor(extract) => Some(extract(request)?),
      Self::Anonymous => None,
    };
    let mut hash = Sha256::new();
    hash.update(b"tako:idempotency:identity");
    match identity {
      None => hash.update([0]),
      Some(identity) => {
        if identity.principal.trim().is_empty()
          || identity
            .tenant
            .as_ref()
            .is_some_and(|tenant| tenant.trim().is_empty())
        {
          return None;
        }
        hash.update([1]);
        hash_field(&mut hash, identity.principal.as_bytes());
        if let Some(tenant) = identity.tenant {
          hash.update([1]);
          hash_field(&mut hash, tenant.as_bytes());
        } else {
          hash.update([0]);
        }
      }
    }
    match scope {
      Scope::KeyOnly => hash.update([0]),
      Scope::MethodAndPath => {
        hash.update([1]);
        hash_field(&mut hash, request.method().as_str().as_bytes());
        hash_field(&mut hash, request.uri().path().as_bytes());
      }
    }
    hash_field(&mut hash, key.as_bytes());
    Some(format!(
      "tako:idempotency:{}",
      URL_SAFE_NO_PAD.encode(hash.finalize())
    ))
  }
}

fn hash_field(hash: &mut Sha256, bytes: &[u8]) {
  hash.update((bytes.len() as u64).to_be_bytes());
  hash.update(bytes);
}
