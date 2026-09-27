use std::sync::Arc;

use crate::stores::IdempotencyStore;
use crate::stores::runtime;

pub(crate) struct InflightGuard {
  pub(crate) store: Arc<dyn IdempotencyStore>,
  pub(crate) key: String,
  pub(crate) lease: String,
  pub(crate) armed: bool,
}

impl Drop for InflightGuard {
  fn drop(&mut self) {
    if !self.armed {
      return;
    }
    let store = self.store.clone();
    let key = self.key.clone();
    let lease = self.lease.clone();
    // Async cleanup is best-effort on cancellation; the backend lease also expires.
    runtime::spawn(async move {
      if let Err(error) = store.remove(&key, &lease).await {
        tracing::warn!(%error, "idempotency lease cleanup failed");
      }
    });
  }
}
