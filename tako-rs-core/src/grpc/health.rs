//! `grpc.health.v1.Health`: register Check, Watch, and List on an ordinary router.

use std::sync::Arc;

use futures_util::Stream;
use scc::HashMap as SccHashMap;
use tokio::sync::watch;

use super::GrpcDeadline;
use super::GrpcRequest;
use super::GrpcResponse;
use super::GrpcServerStream;
use super::GrpcStatus;
use super::GrpcStatusCode;
use crate::router::Router;

mod protocol;
pub use protocol::*;

/// Encoded health schema for reflection registration.
pub const FILE_DESCRIPTOR_SET: &[u8] = include_bytes!("proto/health.bin");

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum ServingStatus {
  Unknown = 0,
  #[default]
  Serving = 1,
  NotServing = 2,
  ServiceUnknown = 3,
}

/// Service statuses; clones share one map.
#[derive(Clone)]
pub struct HealthRegistry {
  inner: Arc<SccHashMap<String, ServingStatus>>,
  updates: Arc<watch::Sender<()>>,
}

impl Default for HealthRegistry {
  fn default() -> Self {
    Self {
      inner: Arc::default(),
      updates: Arc::new(watch::channel(()).0),
    }
  }
}

impl HealthRegistry {
  /// Registers Check, Watch, and List.
  pub fn register(&self, router: &mut Router) {
    let registry = self.clone();
    router.post(
      "/grpc.health.v1.Health/Check",
      move |deadline: Option<GrpcDeadline>, request: GrpcRequest<HealthCheckRequest>| {
        let status = registry.status_of(&request.message.service);
        async move {
          if deadline.is_some_and(|deadline| deadline.remaining().is_zero()) {
            return GrpcResponse::error(GrpcStatusCode::DeadlineExceeded, "deadline exceeded");
          }
          if status == ServingStatus::ServiceUnknown {
            GrpcResponse::error(GrpcStatusCode::NotFound, "unknown service")
          } else {
            GrpcResponse::ok(HealthCheckResponse {
              status: status as i32,
            })
          }
        }
      },
    );
    let registry = self.clone();
    router.post(
      "/grpc.health.v1.Health/Watch",
      move |deadline: Option<GrpcDeadline>, request: GrpcRequest<HealthCheckRequest>| {
        let stream = registry.watch(request.message.service);
        async move { GrpcServerStream::new(stream).with_deadline(deadline) }
      },
    );
    let registry = self.clone();
    router.post(
      "/grpc.health.v1.Health/List",
      move |deadline: Option<GrpcDeadline>, _: GrpcRequest<HealthListRequest>| {
        let statuses = registry
          .snapshot()
          .into_iter()
          .map(|(name, status)| {
            (
              name,
              HealthCheckResponse {
                status: status as i32,
              },
            )
          })
          .collect();
        async move {
          if deadline.is_some_and(|deadline| deadline.remaining().is_zero()) {
            return GrpcResponse::error(GrpcStatusCode::DeadlineExceeded, "deadline exceeded");
          }
          GrpcResponse::ok(HealthListResponse { statuses })
        }
      },
    );
  }

  pub fn new() -> Self {
    Self::default()
  }

  /// Sets the status of `service`; the empty name `""` is the server's overall status.
  pub fn set_status(&self, service: impl Into<String>, status: ServingStatus) {
    // `insert_sync` would keep the first status ever set.
    self.inner.upsert_sync(service.into(), status);
    self.updates.send_replace(());
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

  /// Yields the current status immediately, then changes until dropped.
  /// Unknown services stay subscribed; slow readers see the latest status.
  pub fn watch<S: Into<String>>(
    &self,
    service: S,
  ) -> impl Stream<Item = Result<HealthCheckResponse, GrpcStatus>> + Send + 'static + use<S> {
    let state = (self.clone(), self.updates.subscribe(), service.into(), None);
    futures_util::stream::unfold(
      state,
      |(registry, mut updates, service, mut previous)| async move {
        loop {
          if previous.is_some() {
            updates.changed().await.ok()?;
          }
          let status = registry.status_of(&service);
          if previous != Some(status) {
            previous = Some(status);
            return Some((
              Ok(HealthCheckResponse {
                status: status as i32,
              }),
              (registry, updates, service, previous),
            ));
          }
        }
      },
    )
  }
}
