use std::sync::Arc;

use anyhow::Result;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::router::Router;
use tako_rs_core::signals::Signal;
use tako_rs_core::signals::app_signals;
use tako_rs_core::signals::ids;

/// Translates Tako signals into metric updates; methods run on the signal bus and must not block.
pub trait MetricsBackend: Send + Sync + 'static {
  /// Called once per request at the app level.
  fn on_request_completed(&self, signal: &Signal);

  /// Called once per request that matched a route.
  fn on_route_request_completed(&self, signal: &Signal);

  fn on_connection_opened(&self, signal: &Signal);

  fn on_connection_closed(&self, signal: &Signal);
}

/// Default Prometheus / `OTel` histogram bucket schedule (seconds), tuned for
/// HTTP request latencies between sub-millisecond and ten seconds.
pub const DEFAULT_LATENCY_BUCKETS_SEC: &[f64] = &[
  0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];

pub struct MetricsPlugin<B: MetricsBackend> {
  backend: Arc<B>,
}

impl<B: MetricsBackend> Clone for MetricsPlugin<B> {
  fn clone(&self) -> Self {
    Self {
      backend: Arc::clone(&self.backend),
    }
  }
}

impl<B: MetricsBackend> MetricsPlugin<B> {
  pub fn new(backend: B) -> Self {
    Self {
      backend: Arc::new(backend),
    }
  }
}

impl<B: MetricsBackend> TakoPlugin for MetricsPlugin<B> {
  fn name(&self) -> &'static str {
    "MetricsPlugin"
  }

  fn setup(&self, router: &Router) -> Result<()> {
    let backend = self.backend.clone();
    let app_arbiter = app_signals();

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
}

/// A bounded `transport` label; a per-IP label would create one series per client.
pub(super) fn transport_label(signal: &Signal) -> &'static str {
  if signal.metadata.get("protocol").map(String::as_str) == Some("h3") {
    "h3"
  } else if signal.metadata.get("tls").map(String::as_str) == Some("true") {
    "tls"
  } else if signal.metadata.contains_key("unix_path") {
    "unix"
  } else {
    "tcp"
  }
}

/// The route template, or `unmatched`: raw paths would let scanners create unbounded series.
pub(super) fn route_label(signal: &Signal) -> &str {
  signal
    .metadata
    .get("route")
    .map_or("unmatched", String::as_str)
}
