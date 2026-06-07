//! In-memory session store, stored entry type, expiry policy, and the
//! programmatic revocation handle.

use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use scc::HashMap as SccHashMap;

/// Session expiration policy.
#[derive(Clone, Copy)]
pub struct SessionTtl {
  /// Seconds of inactivity before the session is invalidated.
  pub idle_secs: u64,
  /// Hard cap on total session lifetime regardless of activity. `None` means
  /// only the idle timeout applies.
  pub absolute_secs: Option<u64>,
}

impl Default for SessionTtl {
  fn default() -> Self {
    Self {
      idle_secs: 3_600,
      absolute_secs: Some(86_400),
    }
  }
}

#[derive(Clone)]
pub(crate) struct SessionEntry {
  pub(crate) data: serde_json::Map<String, serde_json::Value>,
  pub(crate) created_at: Instant,
  pub(crate) last_seen_at: Instant,
}

/// Internal session store wrapper. Cloneable handle to the same `SccHashMap`.
#[derive(Clone)]
pub(crate) struct Store(Arc<SccHashMap<String, SessionEntry>>);

impl Store {
  pub(crate) fn new() -> Self {
    Self(Arc::new(SccHashMap::new()))
  }

  pub(crate) fn get(&self, id: &str) -> Option<SessionEntry> {
    self.0.get_sync(id).map(|e| e.clone())
  }

  pub(crate) fn upsert(&self, id: String, entry: SessionEntry) {
    let _ = self.0.upsert_sync(id, entry);
  }

  pub(crate) fn remove(&self, id: &str) {
    let _ = self.0.remove_sync(id);
  }

  fn revoke_all(&self) {
    self.0.clear_sync();
  }

  fn revoke_predicate(&self, mut keep: impl FnMut(&str, &SessionEntry) -> bool) {
    self.0.retain_sync(|k, v| keep(k, v));
  }

  pub(crate) fn retain_expired(&self, ttl: SessionTtl) {
    let now = Instant::now();
    let idle = Duration::from_secs(ttl.idle_secs);
    let absolute = ttl.absolute_secs.map(Duration::from_secs);
    self.0.retain_sync(|_, v| {
      if now.duration_since(v.last_seen_at) > idle {
        return false;
      }
      if let Some(abs) = absolute
        && now.duration_since(v.created_at) > abs
      {
        return false;
      }
      true
    });
  }
}

/// Programmatic store handle returned by [`SessionMiddleware::handle`](super::layer::SessionMiddleware::handle).
#[derive(Clone)]
pub struct SessionStoreHandle {
  pub(crate) store: Store,
}

impl SessionStoreHandle {
  /// Drops every session.
  pub fn revoke_all(&self) {
    self.store.revoke_all();
  }

  /// Drops sessions matching the predicate (returns false to drop).
  pub fn revoke_where<F>(&self, mut pred: F)
  where
    F: FnMut(&str, &serde_json::Map<String, serde_json::Value>) -> bool,
  {
    self.store.revoke_predicate(|k, v| !pred(k, &v.data));
  }
}
