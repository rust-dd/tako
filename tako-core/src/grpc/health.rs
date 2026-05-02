//! `grpc.health.v1.Health` scaffolding.
//!
//! Maintains a per-service health-status map. Handlers / middleware can flip
//! a service from `Serving` to `NotServing` (e.g. when a backing dependency
//! fails) and the `Check` / `Watch` RPC implementations report it.
//!
//! ⚠️ **Status:** like the reflection module, this is the building block.
//! Integrate with a generated `Health` server stub or hand-rolled handler;
//! the registry below is the long-lived state.

use std::sync::Arc;

use scc::HashMap as SccHashMap;

/// Per-service health status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ServingStatus {
  Unknown,
  #[default]
  Serving,
  NotServing,
  ServiceUnknown,
}

/// Health registry shared between handlers and operator hooks.
#[derive(Clone, Default)]
pub struct HealthRegistry {
  inner: Arc<SccHashMap<String, ServingStatus>>,
}

impl HealthRegistry {
  /// New empty registry.
  pub fn new() -> Self {
    Self::default()
  }

  /// Set the status for a service (empty string `""` is the overall status).
  pub fn set_status(&self, service: impl Into<String>, status: ServingStatus) {
    let _ = self.inner.insert_sync(service.into(), status);
  }

  /// Read the status for a service.
  pub fn status_of(&self, service: &str) -> ServingStatus {
    self
      .inner
      .get_sync(service)
      .map(|e| *e.get())
      .unwrap_or(ServingStatus::ServiceUnknown)
  }

  /// Snapshot of every registered service and its status.
  pub fn snapshot(&self) -> Vec<(String, ServingStatus)> {
    let mut out = Vec::new();
    self.inner.iter_sync(|k, v| {
      out.push((k.clone(), *v));
      true
    });
    out
  }
}
