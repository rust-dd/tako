//! Apollo Persisted Queries (APQ) support.
//!
//! APQ flow:
//!
//! 1. Client sends `{extensions: {persistedQuery: {sha256Hash, version: 1}}}`
//!    without a `query` field.
//! 2. Server looks up the hash in a [`PersistedQueryStore`](crate::graphql::apq::PersistedQueryStore).
//!    - **Hit:** populate the request `query` from the cache and execute.
//!    - **Miss:** respond with `PERSISTED_QUERY_NOT_FOUND`. Client retries
//!      with the full `query`.
//! 3. Server caches the `(hash, query)` pair on first full submission.
//!
//! This module exposes the [`PersistedQueryStore`](crate::graphql::apq::PersistedQueryStore) trait, an in-memory
//! implementation, and the [`process`](crate::graphql::apq::process) helper that walks an `async_graphql`
//! request through the lookup-or-store flow. It is a thin wrapper — the
//! actual GraphQL execution still goes through the `async-graphql` schema.

use std::sync::Arc;

use async_trait::async_trait;
use scc::HashMap as SccHashMap;
use sha2::Digest;
use sha2::Sha256;

/// Store backing the persisted-query cache.
#[async_trait]
pub trait PersistedQueryStore: Send + Sync + 'static {
  /// Retrieve a cached query by its SHA-256 hex hash.
  async fn get(&self, hash: &str) -> Option<String>;
  /// Cache a `(hash, query)` pair.
  async fn put(&self, hash: String, query: String);
}

/// Default in-memory store backed by `scc::HashMap`. Rotate by replacing the
/// `Arc` at runtime when you want to evict everything.
#[derive(Clone, Default)]
pub struct MemoryPersistedQueryStore {
  inner: Arc<SccHashMap<String, String>>,
}

impl MemoryPersistedQueryStore {
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl PersistedQueryStore for MemoryPersistedQueryStore {
  async fn get(&self, hash: &str) -> Option<String> {
    self.inner.get_async(hash).await.map(|e| e.get().clone())
  }

  async fn put(&self, hash: String, query: String) {
    let _ = self.inner.insert_async(hash, query).await;
  }
}

/// Errors emitted by the APQ pipeline.
#[derive(Debug, Clone)]
pub enum ApqError {
  /// Client referenced a hash the store does not know — instruct the client
  /// to retry with the full query.
  PersistedQueryNotFound,
  /// Client supplied both a query and a hash but they don't match.
  HashMismatch,
  /// `extensions.persistedQuery.version` was not `1`.
  UnsupportedVersion,
}

impl ApqError {
  /// `PERSISTED_QUERY_NOT_FOUND` is the canonical Apollo extensions code.
  pub fn extensions_code(&self) -> &'static str {
    match self {
      ApqError::PersistedQueryNotFound => "PERSISTED_QUERY_NOT_FOUND",
      ApqError::HashMismatch => "PERSISTED_QUERY_HASH_MISMATCH",
      ApqError::UnsupportedVersion => "PERSISTED_QUERY_UNSUPPORTED_VERSION",
    }
  }
}

/// Compute the lowercase hex SHA-256 of a query string.
pub fn sha256_hash(query: &str) -> String {
  let digest = Sha256::digest(query.as_bytes());
  let mut hex = String::with_capacity(64);
  for b in digest {
    hex.push_str(&format!("{:02x}", b));
  }
  hex
}

/// Process an `async_graphql::Request` against the persisted-query store.
///
/// - When the request carries `extensions.persistedQuery.sha256Hash`:
///   - if `query` is empty: look up the hash in the store; on miss return
///     `PersistedQueryNotFound`.
///   - if `query` is present: verify the hash matches; on success cache it.
/// - When no persisted-query extension is present: pass-through.
#[cfg(feature = "async-graphql")]
pub async fn process(
  mut req: async_graphql::Request,
  store: &dyn PersistedQueryStore,
) -> Result<async_graphql::Request, ApqError> {
  use async_graphql::Value;

  let Some(Value::Object(pq)) = req.extensions.get("persistedQuery").cloned() else {
    return Ok(req);
  };

  let version = pq
    .get("version")
    .and_then(|v| match v {
      Value::Number(n) => n.as_u64(),
      _ => None,
    })
    .unwrap_or(1);
  if version != 1 {
    return Err(ApqError::UnsupportedVersion);
  }

  let hash: Option<String> = pq.get("sha256Hash").and_then(|v| match v {
    Value::String(s) => Some(s.clone()),
    _ => None,
  });

  let Some(hash) = hash else {
    return Ok(req);
  };

  if req.query.is_empty() {
    if let Some(query) = store.get(&hash).await {
      req.query = query;
      Ok(req)
    } else {
      Err(ApqError::PersistedQueryNotFound)
    }
  } else {
    let computed = sha256_hash(&req.query);
    if computed == hash {
      let q = req.query.clone();
      store.put(hash, q).await;
      Ok(req)
    } else {
      Err(ApqError::HashMismatch)
    }
  }
}
