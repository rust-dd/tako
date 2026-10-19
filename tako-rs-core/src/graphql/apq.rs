//! Apollo Persisted Queries: a client may send only a query's `sha256Hash` and, on
//! `PERSISTED_QUERY_NOT_FOUND`, retry with the full query, which is then cached.

use std::sync::Arc;

use async_trait::async_trait;
use scc::HashMap as SccHashMap;
use sha2::Digest;
use sha2::Sha256;

/// Cache of persisted queries keyed by their hash.
#[async_trait]
pub trait PersistedQueryStore: Send + Sync + 'static {
  /// Looks up a query by its lowercase hex SHA-256.
  async fn get(&self, hash: &str) -> Option<String>;
  async fn put(&self, hash: String, query: String);
}

/// In-memory store that drops every entry once full; concurrent `put`s can overshoot the
/// cap by their number.
#[derive(Clone)]
pub struct MemoryPersistedQueryStore {
  inner: Arc<SccHashMap<String, String>>,
  max_entries: usize,
}

impl Default for MemoryPersistedQueryStore {
  fn default() -> Self {
    Self::new()
  }
}

impl MemoryPersistedQueryStore {
  /// Creates a store capped at 1024 entries.
  pub fn new() -> Self {
    Self::with_max_entries(1024)
  }

  /// A `max_entries` of 0 is treated as 1.
  pub fn with_max_entries(max_entries: usize) -> Self {
    Self {
      inner: Arc::new(SccHashMap::new()),
      max_entries: max_entries.max(1),
    }
  }
}

#[async_trait]
impl PersistedQueryStore for MemoryPersistedQueryStore {
  async fn get(&self, hash: &str) -> Option<String> {
    self.inner.get_async(hash).await.map(|e| e.get().clone())
  }

  async fn put(&self, hash: String, query: String) {
    if self.inner.len() >= self.max_entries {
      self.inner.clear_async().await;
    }
    let _ = self.inner.insert_async(hash, query).await;
  }
}

/// Errors emitted by [`process`].
#[derive(Debug, Clone)]
pub enum ApqError {
  /// The hash is not cached; the client should retry with the full query.
  PersistedQueryNotFound,
  /// The request carries both a query and a hash, and they do not match.
  HashMismatch,
  /// `extensions.persistedQuery.version` was not `1`.
  UnsupportedVersion,
}

impl ApqError {
  /// The Apollo `extensions.code` to report, e.g. `PERSISTED_QUERY_NOT_FOUND`.
  pub fn extensions_code(&self) -> &'static str {
    match self {
      ApqError::PersistedQueryNotFound => "PERSISTED_QUERY_NOT_FOUND",
      ApqError::HashMismatch => "PERSISTED_QUERY_HASH_MISMATCH",
      ApqError::UnsupportedVersion => "PERSISTED_QUERY_UNSUPPORTED_VERSION",
    }
  }
}

/// Lowercase hex SHA-256 of `query`.
pub fn sha256_hash(query: &str) -> String {
  use std::fmt::Write as _;
  let digest = Sha256::digest(query.as_bytes());
  let mut hex = String::with_capacity(64);
  for b in digest {
    let _ = write!(&mut hex, "{b:02x}");
  }
  hex
}

/// Fills an empty `query` from `store` by its `persistedQuery` hash, or checks and caches a
/// full one; requests without the extension pass through unchanged.
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
