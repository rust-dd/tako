use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::RwLock;
use scc::HashMap as SccHashMap;
use scc::hash_map::Entry;

use crate::stores::SessionStore;
use crate::stores::StoreResult;
use crate::stores::StoredSession;

struct SessionEntry {
  session: StoredSession,
  expires_at: Instant,
}

#[derive(Default)]
struct Inner {
  entries: SccHashMap<String, SessionEntry>,
  // Rotation and administrative revocation must exclude every other store operation.
  transaction: RwLock<()>,
}

/// In-memory session backend.
#[derive(Default, Clone)]
pub struct MemorySessionStore {
  inner: Arc<Inner>,
}

impl MemorySessionStore {
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
  async fn load(&self, id: &str) -> StoreResult<Option<StoredSession>> {
    let _transaction = self.inner.transaction.read();
    let Some(entry) = self.inner.entries.get_sync(id) else {
      return Ok(None);
    };
    if entry.expires_at <= Instant::now() {
      return Ok(None);
    }
    Ok(Some(entry.session.clone()))
  }

  async fn create(&self, id: &str, data: Vec<u8>, ttl: Duration) -> StoreResult<bool> {
    let _transaction = self.inner.transaction.read();
    Ok(self.insert_fresh(id, data, ttl))
  }

  async fn update(
    &self,
    id: &str,
    version: &str,
    data: Vec<u8>,
    ttl: Duration,
  ) -> StoreResult<bool> {
    let _transaction = self.inner.transaction.read();
    let Some(mut entry) = self.inner.entries.get_sync(id) else {
      return Ok(false);
    };
    if entry.session.version != version || entry.expires_at <= Instant::now() {
      return Ok(false);
    }
    entry.get_mut().session.data = data;
    entry.get_mut().expires_at = Instant::now() + ttl;
    Ok(true)
  }

  async fn rotate(
    &self,
    id: &str,
    version: &str,
    new_id: &str,
    data: Vec<u8>,
    ttl: Duration,
  ) -> StoreResult<bool> {
    let _transaction = self.inner.transaction.write();
    let matches = self.inner.entries.read_sync(id, |_, entry| {
      entry.session.version == version && entry.expires_at > Instant::now()
    });
    if id == new_id || matches != Some(true) || !self.insert_fresh(new_id, data, ttl) {
      return Ok(false);
    }
    self.inner.entries.remove_sync(id);
    Ok(true)
  }

  async fn remove(&self, id: &str, version: &str) -> StoreResult<bool> {
    let _transaction = self.inner.transaction.read();
    Ok(
      self
        .inner
        .entries
        .remove_if_sync(id, |entry| entry.session.version == version)
        .is_some(),
    )
  }

  async fn sweep(&self) -> StoreResult<()> {
    let now = Instant::now();
    let _transaction = self.inner.transaction.read();
    self.inner.entries.retain_sync(|_, v| v.expires_at > now);
    Ok(())
  }
}

impl MemorySessionStore {
  fn insert_fresh(&self, id: &str, data: Vec<u8>, ttl: Duration) -> bool {
    let entry = SessionEntry {
      session: StoredSession {
        data,
        version: uuid::Uuid::new_v4().simple().to_string(),
      },
      expires_at: Instant::now() + ttl,
    };
    match self.inner.entries.entry_sync(id.to_owned()) {
      Entry::Vacant(slot) => {
        slot.insert_entry(entry);
      }
      Entry::Occupied(mut slot) => {
        if slot.get().expires_at > Instant::now() {
          return false;
        }
        *slot.get_mut() = entry;
      }
    }
    true
  }

  pub(crate) fn clear(&self) {
    let _transaction = self.inner.transaction.write();
    self.inner.entries.clear_sync();
  }
  pub(crate) fn retain(&self, mut keep: impl FnMut(&str, &[u8]) -> bool) {
    let _transaction = self.inner.transaction.write();
    self
      .inner
      .entries
      .retain_sync(|key, entry| keep(key, &entry.session.data));
  }
}
