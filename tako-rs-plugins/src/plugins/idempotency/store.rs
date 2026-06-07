//! In-memory idempotency store: cached responses, entry states, and the
//! RAII guard that keeps coalescing waiters from hanging on a dropped handler.

use std::sync::Arc;
use std::time::Instant;

use anyhow::Result;
use bytes::Bytes;
use http::HeaderName;
use http::HeaderValue;
use http::StatusCode;
use scc::HashMap as SccHashMap;
use tokio::sync::Notify;

#[derive(Clone)]
pub(crate) struct CachedResponse {
  pub(crate) status: StatusCode,
  pub(crate) headers: Vec<(HeaderName, HeaderValue)>,
  pub(crate) body: Bytes,
}

#[derive(Clone)]
pub(crate) struct Completed {
  pub(crate) payload_sig: [u8; 20],
  pub(crate) cached: Arc<CachedResponse>,
  pub(crate) expires_at: Instant,
}

pub(crate) enum Entry {
  InFlight {
    payload_sig: [u8; 20],
    notify: Arc<Notify>,
    started: Instant,
  },
  Completed(Completed),
}

#[derive(Clone)]
pub(crate) struct Store(Arc<SccHashMap<String, Entry>>);

/// RAII guard that ensures a registered in-flight entry is cleaned up even if
/// the handler future panics or is dropped before completion. Without this,
/// coalescing waiters parked on `notify.notified()` would never observe a
/// resolution and would hang for the lifetime of the process.
pub(crate) struct InflightGuard {
  store: Store,
  cache_key: String,
  notify: Arc<Notify>,
  armed: bool,
}

impl InflightGuard {
  pub(crate) fn new(store: Store, cache_key: String, notify: Arc<Notify>) -> Self {
    Self {
      store,
      cache_key,
      notify,
      armed: true,
    }
  }

  /// Mark the guard inactive on normal completion paths — the caller has
  /// already either persisted a Completed entry or explicitly removed the
  /// in-flight one.
  pub(crate) fn disarm(&mut self) {
    self.armed = false;
  }
}

impl Drop for InflightGuard {
  fn drop(&mut self) {
    if self.armed {
      self.store.remove(&self.cache_key);
      self.notify.notify_waiters();
    }
  }
}

impl Store {
  pub(crate) fn new() -> Self {
    Self(Arc::new(SccHashMap::new()))
  }

  pub(crate) fn get(&self, k: &str) -> Option<Entry> {
    self.0.get_sync(k).map(|e| match &*e {
      Entry::InFlight {
        payload_sig,
        notify,
        started,
      } => Entry::InFlight {
        payload_sig: *payload_sig,
        notify: notify.clone(),
        started: *started,
      },
      Entry::Completed(c) => Entry::Completed(c.clone()),
    })
  }

  /// Atomically install a fresh `InFlight` entry for `k`, or return the
  /// entry already present.
  ///
  /// This is the only race-safe alternative to a separate `get()` followed
  /// by `insert_*()`: with two pre-existing primitives, two concurrent
  /// requests for the same key could both see `None` and both call
  /// `insert_*` — duplicating handler work, losing one of the notifiers,
  /// and (after PPL-03) silently overwriting the first writer's Completed
  /// entry. `entry_sync` collapses the check-and-install into one atomic
  /// step on the same bucket lock.
  pub(crate) fn install_inflight_or_get_existing(
    &self,
    k: String,
    payload_sig: [u8; 20],
  ) -> Result<Arc<Notify>, Entry> {
    use scc::hash_map::Entry as MapEntry;
    match self.0.entry_sync(k) {
      MapEntry::Vacant(v) => {
        let notify = Arc::new(Notify::new());
        v.insert_entry(Entry::InFlight {
          payload_sig,
          notify: notify.clone(),
          started: Instant::now(),
        });
        Ok(notify)
      }
      MapEntry::Occupied(o) => Err(match o.get() {
        Entry::Completed(c) => Entry::Completed(c.clone()),
        Entry::InFlight {
          payload_sig,
          notify,
          started,
        } => Entry::InFlight {
          payload_sig: *payload_sig,
          notify: notify.clone(),
          started: *started,
        },
      }),
    }
  }

  pub(crate) fn complete(&self, k: String, completed: Completed) {
    // MUST be `upsert_sync`: the key already holds the matching InFlight
    // entry (planted by `install_inflight_or_get_existing` before the
    // handler ran). `insert_sync` would no-op on collision, leaving the
    // cache filled with InFlight forever and forcing every replay through
    // the 409 conflict path — i.e. the whole idempotency store would be
    // dead.
    self.0.upsert_sync(k, Entry::Completed(completed));
  }

  pub(crate) fn remove(&self, k: &str) {
    let _ = self.0.remove_sync(k);
  }

  pub(crate) fn retain_expired(&self) {
    let now = Instant::now();
    self.0.retain_sync(|_, v| match v {
      Entry::Completed(c) => c.expires_at > now,
      Entry::InFlight { .. } => true,
    });
  }
}
