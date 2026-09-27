use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use scc::HashMap as SccHashMap;

use crate::stores::SessionStore;
use crate::stores::StoreResult;
#[derive(Clone)]
struct SessionEntry {
  data: Vec<u8>,
  expires_at: Instant,
}

/// In-memory session backend.
#[derive(Default, Clone)]
pub struct MemorySessionStore {
  inner: Arc<SccHashMap<String, SessionEntry>>,
}

impl MemorySessionStore {
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
  async fn load(&self, id: &str) -> StoreResult<Option<Vec<u8>>> {
    let Some(entry) = self.inner.get_async(id).await else {
      return Ok(None);
    };
    if entry.expires_at <= Instant::now() {
      return Ok(None);
    }
    Ok(Some(entry.data.clone()))
  }

  async fn store(&self, id: &str, data: Vec<u8>, ttl: Duration) -> StoreResult<()> {
    let entry = SessionEntry {
      data,
      expires_at: Instant::now() + ttl,
    };
    let _ = self.inner.upsert_async(id.to_string(), entry).await;
    Ok(())
  }

  async fn remove(&self, id: &str) -> StoreResult<bool> {
    Ok(self.inner.remove_async(id).await.is_some())
  }

  async fn sweep(&self) -> StoreResult<()> {
    let now = Instant::now();
    self.inner.retain_async(|_, v| v.expires_at > now).await;
    Ok(())
  }
}

impl MemorySessionStore {
  pub(crate) fn clear(&self) {
    self.inner.clear_sync();
  }
  pub(crate) fn retain(&self, mut keep: impl FnMut(&str, &[u8]) -> bool) {
    self.inner.retain_sync(|key, entry| keep(key, &entry.data));
  }
}
