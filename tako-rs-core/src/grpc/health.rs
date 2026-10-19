//! Per-service status map for a `grpc.health.v1.Health` service; serve it from your own
//! `Check` and `Watch` handlers.

use std::sync::Arc;

use scc::HashMap as SccHashMap;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServingStatus {
  Unknown,
  #[default]
  Serving,
  NotServing,
  ServiceUnknown,
}

/// Service statuses; clones share one map.
#[derive(Clone, Default)]
pub struct HealthRegistry {
  inner: Arc<SccHashMap<String, ServingStatus>>,
}

impl HealthRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  /// Sets the status of `service`; the empty name `""` is the server's overall status.
  pub fn set_status(&self, service: impl Into<String>, status: ServingStatus) {
    // `insert_sync` would keep the first status ever set.
    self.inner.upsert_sync(service.into(), status);
  }

  /// `ServiceUnknown` for a service that was never set.
  pub fn status_of(&self, service: &str) -> ServingStatus {
    self
      .inner
      .get_sync(service)
      .map_or(ServingStatus::ServiceUnknown, |e| *e.get())
  }

  pub fn snapshot(&self) -> Vec<(String, ServingStatus)> {
    let mut out = Vec::new();
    self.inner.iter_sync(|k, v| {
      out.push((k.clone(), *v));
      true
    });
    out
  }
}
