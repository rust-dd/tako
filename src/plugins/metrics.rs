#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Metrics and tracing plugin for integrating Tako's signal system with
//! backends like Prometheus or OpenTelemetry.
//!
//! This plugin listens to application-level and route-level signals and
//! updates metrics using an injected backend implementation. When the
//! `metrics-prometheus` or `metrics-opentelemetry` features are enabled,
//! a concrete backend is provided based on the selected feature, while
//! the core plugin logic remains backend-agnostic.

use anyhow::Result;
use std::sync::Arc;

#[cfg(feature = "signals")]
use crate::signals::{Signal, app_events, ids};
use crate::{plugins::TakoPlugin, router::Router};

#[cfg(feature = "metrics-prometheus")]
use crate::{Method, extractors::state::State, responder::Responder};
#[cfg(feature = "metrics-prometheus")]
use prometheus::{Encoder, Registry, TextEncoder};

/// Common interface for metrics backends used by the metrics plugin.
///
/// Backend implementations translate Tako signals into metrics updates
/// or tracing events in external systems.
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

/// Metrics plugin that subscribes to Tako's signal bus and forwards
/// events to a configurable metrics backend.
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
  /// Creates a new metrics plugin using the provided backend.
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

  #[cfg(feature = "signals")]
  fn setup(&self, _router: &Router) -> Result<()> {
    let backend = self.backend.clone();
    let app_arbiter = app_events();

    // App-level request.completed metrics
    app_arbiter.on(ids::REQUEST_COMPLETED, move |signal: Signal| {
      let backend = backend.clone();
      async move {
        backend.on_request_completed(&signal);
      }
    });

    // Connection lifetime metrics
    let backend_conn = self.backend.clone();
    app_arbiter.on(ids::CONNECTION_OPENED, move |signal: Signal| {
      let backend = backend_conn.clone();
      async move {
        backend.on_connection_opened(&signal);
      }
    });

    let backend_close = self.backend.clone();
    app_arbiter.on(ids::CONNECTION_CLOSED, move |signal: Signal| {
      let backend = backend_close.clone();
      async move {
        backend.on_connection_closed(&signal);
      }
    });

    // Route-level request.completed metrics via prefix subscription
    let backend_route = self.backend.clone();
    let mut rx = app_arbiter.subscribe_prefix("route.request.");
    tokio::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        backend_route.on_route_request_completed(&signal);
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

/// Prometheus backend implementation.
#[cfg(feature = "metrics-prometheus")]
pub mod prometheus_backend {
  use super::{MetricsBackend, Signal};
  use prometheus::{IntCounterVec, Opts, Registry};
  use std::sync::Arc;

  /// Basic Prometheus metrics backend that tracks HTTP request counts
  /// and connection counts using labels for method, path, and status.
  pub struct PrometheusMetricsBackend {
    registry: Registry,
    http_requests_total: IntCounterVec,
    http_route_requests_total: IntCounterVec,
    connections_opened_total: IntCounterVec,
    connections_closed_total: IntCounterVec,
  }

  impl PrometheusMetricsBackend {
    pub fn new(registry: Registry) -> Self {
      let http_requests_total = IntCounterVec::new(
        Opts::new("tako_http_requests_total", "Total HTTP requests completed"),
        &["method", "path", "status"],
      )
      .expect("failed to create http_requests_total metric");

      let http_route_requests_total = IntCounterVec::new(
        Opts::new(
          "tako_route_requests_total",
          "Total route-level HTTP requests completed",
        ),
        &["method", "path", "status"],
      )
      .expect("failed to create route_requests_total metric");

      let connections_opened_total = IntCounterVec::new(
        Opts::new("tako_connections_opened_total", "Total connections opened"),
        &["remote_addr"],
      )
      .expect("failed to create connections_opened_total metric");

      let connections_closed_total = IntCounterVec::new(
        Opts::new("tako_connections_closed_total", "Total connections closed"),
        &["remote_addr"],
      )
      .expect("failed to create connections_closed_total metric");

      registry
        .register(Box::new(http_requests_total.clone()))
        .expect("failed to register http_requests_total");
      registry
        .register(Box::new(http_route_requests_total.clone()))
        .expect("failed to register http_route_requests_total");
      registry
        .register(Box::new(connections_opened_total.clone()))
        .expect("failed to register connections_opened_total");
      registry
        .register(Box::new(connections_closed_total.clone()))
        .expect("failed to register connections_closed_total");

      Self {
        registry,
        http_requests_total,
        http_route_requests_total,
        connections_opened_total,
        connections_closed_total,
      }
    }

    pub fn registry(&self) -> &Registry {
      &self.registry
    }
  }

  impl MetricsBackend for Arc<PrometheusMetricsBackend> {
    fn on_request_completed(&self, signal: &Signal) {
      let method = signal
        .metadata
        .get("method")
        .map(String::as_str)
        .unwrap_or("");
      let path = signal
        .metadata
        .get("path")
        .map(String::as_str)
        .unwrap_or("");
      let status = signal
        .metadata
        .get("status")
        .map(String::as_str)
        .unwrap_or("");
      self
        .http_requests_total
        .with_label_values(&[method, path, status])
        .inc();
    }

    fn on_route_request_completed(&self, signal: &Signal) {
      let method = signal
        .metadata
        .get("method")
        .map(String::as_str)
        .unwrap_or("");
      let path = signal
        .metadata
        .get("path")
        .map(String::as_str)
        .unwrap_or("");
      let status = signal
        .metadata
        .get("status")
        .map(String::as_str)
        .unwrap_or("");
      self
        .http_route_requests_total
        .with_label_values(&[method, path, status])
        .inc();
    }

    fn on_connection_opened(&self, signal: &Signal) {
      let addr = signal
        .metadata
        .get("remote_addr")
        .map(String::as_str)
        .unwrap_or("");
      self
        .connections_opened_total
        .with_label_values(&[addr])
        .inc();
    }

    fn on_connection_closed(&self, signal: &Signal) {
      let addr = signal
        .metadata
        .get("remote_addr")
        .map(String::as_str)
        .unwrap_or("");
      self
        .connections_closed_total
        .with_label_values(&[addr])
        .inc();
    }
  }
}

/// OpenTelemetry backend implementation.
#[cfg(feature = "metrics-opentelemetry")]
pub mod opentelemetry_backend {
  use super::{MetricsBackend, Signal};
  use opentelemetry::KeyValue;
  use opentelemetry::metrics::{Counter, Meter};

  /// Basic OpenTelemetry metrics backend that records counters using the
  /// global meter provider. Users are expected to configure an exporter
  /// (e.g. OTLP, Prometheus) separately.
  pub struct OtelMetricsBackend {
    http_requests_total: Counter<u64>,
    http_route_requests_total: Counter<u64>,
    connections_opened_total: Counter<u64>,
    connections_closed_total: Counter<u64>,
  }

  impl OtelMetricsBackend {
    pub fn new(meter: Meter) -> Self {
      let http_requests_total = meter.u64_counter("tako_http_requests_total").init();
      let http_route_requests_total = meter.u64_counter("tako_route_requests_total").init();
      let connections_opened_total = meter.u64_counter("tako_connections_opened_total").init();
      let connections_closed_total = meter.u64_counter("tako_connections_closed_total").init();

      Self {
        http_requests_total,
        http_route_requests_total,
        connections_opened_total,
        connections_closed_total,
      }
    }
  }

  impl MetricsBackend for OtelMetricsBackend {
    fn on_request_completed(&self, signal: &Signal) {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let path = signal.metadata.get("path").cloned().unwrap_or_default();
      let status = signal.metadata.get("status").cloned().unwrap_or_default();
      self.http_requests_total.add(
        1,
        &[
          KeyValue::new("method", method),
          KeyValue::new("path", path),
          KeyValue::new("status", status),
        ],
      );
    }

    fn on_route_request_completed(&self, signal: &Signal) {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let path = signal.metadata.get("path").cloned().unwrap_or_default();
      let status = signal.metadata.get("status").cloned().unwrap_or_default();
      self.http_route_requests_total.add(
        1,
        &[
          KeyValue::new("method", method),
          KeyValue::new("path", path),
          KeyValue::new("status", status),
        ],
      );
    }

    fn on_connection_opened(&self, signal: &Signal) {
      let addr = signal
        .metadata
        .get("remote_addr")
        .cloned()
        .unwrap_or_default();
      self
        .connections_opened_total
        .add(1, &[KeyValue::new("remote_addr", addr)]);
    }

    fn on_connection_closed(&self, signal: &Signal) {
      let addr = signal
        .metadata
        .get("remote_addr")
        .cloned()
        .unwrap_or_default();
      self
        .connections_closed_total
        .add(1, &[KeyValue::new("remote_addr", addr)]);
    }
  }
}

#[cfg(feature = "metrics-prometheus")]
#[derive(Clone)]
pub struct PrometheusMetricsConfig {
  /// HTTP path where the Prometheus scrape endpoint will be exposed.
  pub endpoint_path: String,
}

#[cfg(feature = "metrics-prometheus")]
impl Default for PrometheusMetricsConfig {
  fn default() -> Self {
    Self {
      endpoint_path: "/metrics".to_string(),
    }
  }
}

#[cfg(feature = "metrics-prometheus")]
impl PrometheusMetricsConfig {
  /// Installs a Prometheus metrics backend and a scrape endpoint on the router.
  pub fn install(self, router: &mut Router) -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    let backend = prometheus_backend::PrometheusMetricsBackend::new((*registry).clone());
    let plugin = MetricsPlugin::new(Arc::new(backend));

    router.plugin(plugin);
    router.state(registry.clone());

    let path = self.endpoint_path;
    router.route(Method::GET, &path, prometheus_metrics_handler);

    registry
  }
}

#[cfg(feature = "metrics-prometheus")]
async fn prometheus_metrics_handler(State(registry): State<Arc<Registry>>) -> impl Responder {
  let encoder = TextEncoder::new();
  let metric_families = registry.gather();

  let mut buf = Vec::new();
  if encoder.encode(&metric_families, &mut buf).is_err() {
    return "failed to encode metrics".to_string();
  }

  String::from_utf8(buf).unwrap_or_default()
}

#[cfg(feature = "metrics-opentelemetry")]
#[derive(Clone)]
pub struct OtelMetricsConfig {
  /// Name for the OpenTelemetry meter used by Tako.
  pub meter_name: &'static str,
}

#[cfg(feature = "metrics-opentelemetry")]
impl Default for OtelMetricsConfig {
  fn default() -> Self {
    Self { meter_name: "tako" }
  }
}

#[cfg(feature = "metrics-opentelemetry")]
impl OtelMetricsConfig {
  /// Installs an OpenTelemetry metrics backend using the global meter provider.
  pub fn install(self, router: &mut Router) {
    use opentelemetry::global;

    let meter = global::meter(self.meter_name);
    let backend = opentelemetry_backend::OtelMetricsBackend::new(meter);
    let plugin = MetricsPlugin::new(backend);

    router.plugin(plugin);
  }
}
