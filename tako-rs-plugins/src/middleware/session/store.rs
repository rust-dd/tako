use std::sync::Arc;

use crate::stores::memory::MemorySessionStore;

/// Session expiration policy.
#[derive(Clone, Copy)]
pub struct SessionTtl {
  /// Maximum inactivity in seconds.
  pub idle_secs: u64,
  /// Maximum lifetime irrespective of activity. `None` disables the absolute cap.
  pub absolute_secs: Option<u64>,
}
impl Default for SessionTtl {
  fn default() -> Self {
    Self {
      idle_secs: 3600,
      absolute_secs: Some(86400),
    }
  }
}

#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct SessionRecord {
  pub(crate) data: serde_json::Map<String, serde_json::Value>,
  pub(crate) created_at: u64,
}

/// Revocation handle for the default memory backend.
///
/// Custom stores expose their own administrative revocation operations.
#[derive(Clone)]
pub struct SessionStoreHandle {
  pub(crate) store: Arc<MemorySessionStore>,
}
impl SessionStoreHandle {
  /// Remove every session in this backend.
  pub fn revoke_all(&self) {
    self.store.clear();
  }

  /// Remove sessions for which `predicate` returns true.
  pub fn revoke_where<F>(&self, mut predicate: F)
  where
    F: FnMut(&str, &serde_json::Map<String, serde_json::Value>) -> bool,
  {
    self.store.retain(|key, bytes| {
      serde_json::from_slice::<SessionRecord>(bytes)
        .is_ok_and(|record| !predicate(key, &record.data))
    });
  }
}
