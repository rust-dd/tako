use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use scc::HashMap as SccHashMap;
use scc::hash_map::Entry;

use crate::stores::IdempotencyBegin;
use crate::stores::IdempotencyEntry;
use crate::stores::IdempotencyStore;
use crate::stores::StoreResult;

struct Record {
  entry: IdempotencyEntry,
  lease: String,
  notify: Arc<tokio::sync::Notify>,
  expires: Instant,
}

/// In-memory idempotency leases; pending ownership expires after 300 seconds by default.
#[derive(Clone)]
pub struct MemoryIdempotencyStore {
  inner: Arc<SccHashMap<String, Record>>,
  inflight_ttl: Duration,
}
impl Default for MemoryIdempotencyStore {
  fn default() -> Self {
    Self {
      inner: Arc::new(SccHashMap::new()),
      inflight_ttl: Duration::from_secs(300),
    }
  }
}
impl MemoryIdempotencyStore {
  pub fn new() -> Self {
    Self::default()
  }
  /// Set the pending lease lifetime above the maximum handler duration.
  pub fn with_inflight_ttl(mut self, ttl: Duration) -> Self {
    self.inflight_ttl = ttl;
    self
  }
}
#[async_trait]
impl IdempotencyStore for MemoryIdempotencyStore {
  async fn get(&self, key: &str) -> StoreResult<Option<IdempotencyEntry>> {
    let Some(record) = self.inner.get_async(key).await else {
      return Ok(None);
    };
    Ok((record.expires > Instant::now()).then(|| record.entry.clone()))
  }
  async fn begin(&self, key: &str, payload_sig: [u8; 32]) -> StoreResult<IdempotencyBegin> {
    let lease = uuid::Uuid::new_v4().simple().to_string();
    let record = Record {
      lease: lease.clone(),
      notify: Arc::new(tokio::sync::Notify::new()),
      expires: Instant::now() + self.inflight_ttl,
      entry: IdempotencyEntry {
        status: 0,
        headers: Vec::new(),
        body: bytes::Bytes::new(),
        payload_sig,
        completed: false,
      },
    };
    match self.inner.entry_async(key.to_owned()).await {
      Entry::Vacant(slot) => {
        slot.insert_entry(record);
      }
      Entry::Occupied(mut slot) => {
        if slot.get().expires > Instant::now() {
          return Ok(IdempotencyBegin::Existing(slot.get().entry.clone()));
        }
        *slot.get_mut() = record;
      }
    }
    Ok(IdempotencyBegin::Acquired(lease))
  }
  async fn complete(
    &self,
    key: &str,
    lease: &str,
    entry: IdempotencyEntry,
    ttl: Duration,
  ) -> StoreResult<bool> {
    let Some(mut record) = self.inner.get_async(key).await else {
      return Ok(false);
    };
    if record.lease != lease || record.expires <= Instant::now() || record.entry.completed {
      return Ok(false);
    }
    let notify = record.notify.clone();
    record.get_mut().entry = entry;
    record.get_mut().entry.completed = true;
    record.get_mut().expires = Instant::now() + ttl;
    drop(record);
    notify.notify_waiters();
    Ok(true)
  }
  async fn remove(&self, key: &str, lease: &str) -> StoreResult<bool> {
    if let Some((_, record)) = self
      .inner
      .remove_if_async(key, |record| record.lease == lease)
      .await
    {
      record.notify.notify_waiters();
      Ok(true)
    } else {
      Ok(false)
    }
  }

  async fn wait(
    &self,
    key: &str,
    timeout: Option<Duration>,
  ) -> StoreResult<Option<IdempotencyEntry>> {
    let Some(record) = self.inner.get_async(key).await else {
      return Ok(None);
    };
    let notify = record.notify.clone();
    let remaining = record.expires.saturating_duration_since(Instant::now());
    drop(record);
    let mut notified = std::pin::pin!(notify.notified());
    notified.as_mut().enable();
    let entry = self.get(key).await?;
    if entry.as_ref().is_none_or(|entry| entry.completed) {
      return Ok(entry);
    }
    let delay = timeout.map_or(remaining, |timeout| timeout.min(remaining));
    let sleep = std::pin::pin!(crate::stores::runtime::sleep(delay));
    let _ = futures_util::future::select(notified, sleep).await;
    self.get(key).await
  }

  async fn sweep(&self) -> StoreResult<()> {
    self
      .inner
      .retain_async(|_, record| record.expires > Instant::now())
      .await;
    Ok(())
  }
}
