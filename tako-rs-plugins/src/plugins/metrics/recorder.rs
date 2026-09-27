//! Backend-agnostic core of the metrics plugin: the [`MetricsBackend`] trait,
//! the default latency bucket schedule, and the [`MetricsPlugin`] that wires
//! Tako's signal bus to a configurable backend.

#[cfg(feature = "signals")]
use std::sync::Arc;

#[cfg(feature = "signals")]
use anyhow::Result;
#[cfg(feature = "signals")]
use tako_rs_core::plugins::TakoPlugin;
#[cfg(feature = "signals")]
use tako_rs_core::router::Router;
#[cfg(feature = "signals")]
use tako_rs_core::signals::Signal;
#[cfg(feature = "signals")]
use tako_rs_core::signals::app_events;
#[cfg(feature = "signals")]
use tako_rs_core::signals::ids;

/// Common interface for metrics backends used by the metrics plugin.
///
/// Backend implementations translate Tako signals into metrics updates
/// or tracing events in external systems.
#[cfg(feature = "signals")]
pub trait MetricsBackend: Send + Sync + 'static {
  /// Called when a request is completed at the app level.
  fn on_request_completed(&self, signal: &Signal);

  /// Called when a route-level request is completed.
  fn on_route_request_completed(&self, signal: &Signal);

  /// Called when a connection is opened.
  fn on_connection_opened(&self, signal: &Signal);

  /// Called when a connection is closed.
  fn on_connection_closed(&self, signal: &Signal);
}

/// Default Prometheus / `OTel` histogram bucket schedule (seconds), tuned for
/// HTTP request latencies between sub-millisecond and ten seconds.
pub const DEFAULT_LATENCY_BUCKETS_SEC: &[f64] = &[
  0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

/// Metrics plugin that subscribes to Tako's signal bus and forwards
/// events to a configurable metrics backend.
#[cfg(feature = "signals")]
pub struct MetricsPlugin<B: MetricsBackend> {
  backend: Arc<B>,
}

#[cfg(feature = "signals")]
impl<B: MetricsBackend> Clone for MetricsPlugin<B> {
  fn clone(&self) -> Self {
    Self {
      backend: Arc::clone(&self.backend),
    }
  }
}

#[cfg(feature = "signals")]
impl<B: MetricsBackend> MetricsPlugin<B> {
  /// Creates a new metrics plugin using the provided backend.
  pub fn new(backend: B) -> Self {
    Self {
      backend: Arc::new(backend),
    }
  }
}

#[cfg(feature = "signals")]
impl<B: MetricsBackend> TakoPlugin for MetricsPlugin<B> {
  fn name(&self) -> &'static str {
    "MetricsPlugin"
  }

  #[cfg(feature = "signals")]
  fn setup(&self, router: &Router) -> Result<()> {
    let backend = self.backend.clone();
    let app_arbiter = app_events();

    router
      .signals()
      .on(ids::REQUEST_COMPLETED, move |signal: Signal| {
        let backend = backend.clone();
        async move {
          backend.on_request_completed(&signal);
        }
      });

    let backend_conn = Arc::downgrade(&self.backend);
    app_arbiter.on(ids::CONNECTION_OPENED, move |signal: Signal| {
      let backend = backend_conn.upgrade();
      async move {
        if let Some(backend) = backend {
          backend.on_connection_opened(&signal);
        }
      }
    });

    let backend_close = Arc::downgrade(&self.backend);
    app_arbiter.on(ids::CONNECTION_CLOSED, move |signal: Signal| {
      let backend = backend_close.upgrade();
      async move {
        if let Some(backend) = backend {
          backend.on_connection_closed(&signal);
        }
      }
    });

    let backend_route = self.backend.clone();
    router
      .signals()
      .on(ids::ROUTE_REQUEST_COMPLETED, move |signal| {
        let backend = backend_route.clone();
        async move {
          backend.on_route_request_completed(&signal);
        }
      });

    Ok(())
  }

  #[cfg(not(feature = "signals"))]
  fn setup(&self, _router: &Router) -> Result<()> {
    // Metrics plugin is a no-op when signals are disabled.
    Ok(())
  }
}
