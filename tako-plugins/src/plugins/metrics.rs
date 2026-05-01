#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Metrics and tracing plugin for integrating Tako's signal system with
//! backends like Prometheus or OpenTelemetry.
//!
//! This plugin listens to application-level and route-level signals and
//! updates metrics using an injected backend implementation. When the
//! `metrics-prometheus` or `metrics-opentelemetry` features are enabled,
//! a concrete backend is provided based on the selected feature, while
//! the core plugin logic remains backend-agnostic.

use std::sync::Arc;

use anyhow::Result;
#[cfg(feature = "metrics-prometheus")]
use prometheus::Encoder;
#[cfg(feature = "metrics-prometheus")]
use prometheus::Registry;
#[cfg(feature = "metrics-prometheus")]
use prometheus::TextEncoder;
#[cfg(feature = "metrics-prometheus")]
use tako_core::Method;
use tako_core::plugins::TakoPlugin;
#[cfg(feature = "metrics-prometheus")]
use tako_core::responder::Responder;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::Signal;
#[cfg(feature = "signals")]
use tako_core::signals::app_events;
#[cfg(feature = "signals")]
use tako_core::signals::ids;
#[cfg(feature = "metrics-prometheus")]
use tako_extractors::state::State;

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

/// Default Prometheus / OTel histogram bucket schedule (seconds), tuned for
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
    #[cfg(not(feature = "compio"))]
    tokio::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        backend_route.on_route_request_completed(&signal);
      }
    });

    #[cfg(feature = "compio")]
    compio::runtime::spawn(async move {
      while let Ok(signal) = rx.recv().await {
        backend_route.on_route_request_completed(&signal);
      }
    })
    .detach();

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
  use std::sync::Arc;

  use prometheus::HistogramOpts;
  use prometheus::HistogramVec;
  use prometheus::IntCounterVec;
  use prometheus::Opts;
  use prometheus::Registry;

  use super::DEFAULT_LATENCY_BUCKETS_SEC;
  use super::MetricsBackend;
  use super::Signal;

  /// Derives a low-cardinality `transport` label from a connection signal.
  ///
  /// Replaces the per-IP `remote_addr` label which would otherwise produce
  /// one Prometheus series per client IP.
  fn transport_label(signal: &Signal) -> &'static str {
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

  /// Prefer the matched route template (`route`) over the raw URI path (`path`)
  /// to keep label cardinality bounded by the number of registered routes.
  fn route_label(signal: &Signal) -> &str {
    signal
      .metadata
      .get("route")
      .or_else(|| signal.metadata.get("path"))
      .map(String::as_str)
      .unwrap_or("")
  }

  /// Basic Prometheus metrics backend that tracks HTTP request counts
  /// and connection counts using labels for method, route, and status.
  pub struct PrometheusMetricsBackend {
    registry: Registry,
    http_requests_total: IntCounterVec,
    http_route_requests_total: IntCounterVec,
    http_request_duration: HistogramVec,
    connections_opened_total: IntCounterVec,
    connections_closed_total: IntCounterVec,
  }

  impl PrometheusMetricsBackend {
    /// Builds the backend with the default latency buckets.
    pub fn new(registry: Registry) -> Self {
      Self::with_buckets(registry, DEFAULT_LATENCY_BUCKETS_SEC.to_vec())
    }

    /// Builds the backend with a caller-supplied latency bucket schedule.
    pub fn with_buckets(registry: Registry, buckets: Vec<f64>) -> Self {
      // Route-template-based labels keep cardinality bounded by route count;
      // raw path labels would explode under `/users/:id`-style traffic.
      let http_requests_total = IntCounterVec::new(
        Opts::new("tako_http_requests_total", "Total HTTP requests completed"),
        &["method", "route", "status"],
      )
      .expect("failed to create http_requests_total metric");

      let http_route_requests_total = IntCounterVec::new(
        Opts::new(
          "tako_route_requests_total",
          "Total route-level HTTP requests completed",
        ),
        &["method", "route", "status"],
      )
      .expect("failed to create route_requests_total metric");

      let http_request_duration = HistogramVec::new(
        HistogramOpts::new(
          "tako_http_request_duration_seconds",
          "End-to-end HTTP request duration",
        )
        .buckets(buckets),
        &["method", "route", "status"],
      )
      .expect("failed to create http_request_duration metric");

      // `transport` is bounded (tcp/tls/h3/unix); `remote_addr` was unbounded.
      let connections_opened_total = IntCounterVec::new(
        Opts::new("tako_connections_opened_total", "Total connections opened"),
        &["transport"],
      )
      .expect("failed to create connections_opened_total metric");

      let connections_closed_total = IntCounterVec::new(
        Opts::new("tako_connections_closed_total", "Total connections closed"),
        &["transport"],
      )
      .expect("failed to create connections_closed_total metric");

      registry
        .register(Box::new(http_requests_total.clone()))
        .expect("failed to register http_requests_total");
      registry
        .register(Box::new(http_route_requests_total.clone()))
        .expect("failed to register http_route_requests_total");
      registry
        .register(Box::new(http_request_duration.clone()))
        .expect("failed to register http_request_duration");
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
        http_request_duration,
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
      let route = route_label(signal);
      let status = signal
        .metadata
        .get("status")
        .map(String::as_str)
        .unwrap_or("");
      self
        .http_requests_total
        .with_label_values(&[method, route, status])
        .inc();
      // Histogram observation: the `duration_us` metadata is emitted by
      // upstream signal sites when latency tracking is enabled. Microseconds
      // are converted to seconds (Prometheus convention) before observation.
      if let Some(d_us) = signal
        .metadata
        .get("duration_us")
        .and_then(|s| s.parse::<u64>().ok())
      {
        self
          .http_request_duration
          .with_label_values(&[method, route, status])
          .observe((d_us as f64) / 1_000_000.0);
      }
    }

    fn on_route_request_completed(&self, signal: &Signal) {
      let method = signal
        .metadata
        .get("method")
        .map(String::as_str)
        .unwrap_or("");
      let route = route_label(signal);
      let status = signal
        .metadata
        .get("status")
        .map(String::as_str)
        .unwrap_or("");
      self
        .http_route_requests_total
        .with_label_values(&[method, route, status])
        .inc();
    }

    fn on_connection_opened(&self, signal: &Signal) {
      let transport = transport_label(signal);
      self
        .connections_opened_total
        .with_label_values(&[transport])
        .inc();
    }

    fn on_connection_closed(&self, signal: &Signal) {
      let transport = transport_label(signal);
      self
        .connections_closed_total
        .with_label_values(&[transport])
        .inc();
    }
  }
}

/// OpenTelemetry backend implementation.
#[cfg(feature = "metrics-opentelemetry")]
pub mod opentelemetry_backend {
  use opentelemetry::KeyValue;
  use opentelemetry::metrics::Counter;
  use opentelemetry::metrics::Meter;

  use super::MetricsBackend;
  use super::Signal;

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
      let http_requests_total = meter.u64_counter("tako_http_requests_total").build();
      let http_route_requests_total = meter.u64_counter("tako_route_requests_total").build();
      let connections_opened_total = meter.u64_counter("tako_connections_opened_total").build();
      let connections_closed_total = meter.u64_counter("tako_connections_closed_total").build();

      Self {
        http_requests_total,
        http_route_requests_total,
        connections_opened_total,
        connections_closed_total,
      }
    }
  }

  /// Derives a low-cardinality `transport` label from a connection signal.
  fn transport_label(signal: &Signal) -> &'static str {
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

  /// Prefer the matched route template (`route`) over the raw URI path (`path`).
  fn route_label(signal: &Signal) -> String {
    signal
      .metadata
      .get("route")
      .or_else(|| signal.metadata.get("path"))
      .cloned()
      .unwrap_or_default()
  }

  impl MetricsBackend for OtelMetricsBackend {
    fn on_request_completed(&self, signal: &Signal) {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let route = route_label(signal);
      let status = signal.metadata.get("status").cloned().unwrap_or_default();
      self.http_requests_total.add(
        1,
        &[
          KeyValue::new("method", method),
          KeyValue::new("route", route),
          KeyValue::new("status", status),
        ],
      );
    }

    fn on_route_request_completed(&self, signal: &Signal) {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let route = route_label(signal);
      let status = signal.metadata.get("status").cloned().unwrap_or_default();
      self.http_route_requests_total.add(
        1,
        &[
          KeyValue::new("method", method),
          KeyValue::new("route", route),
          KeyValue::new("status", status),
        ],
      );
    }

    fn on_connection_opened(&self, signal: &Signal) {
      self
        .connections_opened_total
        .add(1, &[KeyValue::new("transport", transport_label(signal))]);
    }

    fn on_connection_closed(&self, signal: &Signal) {
      self
        .connections_closed_total
        .add(1, &[KeyValue::new("transport", transport_label(signal))]);
    }
  }
}

#[cfg(feature = "metrics-prometheus")]
#[derive(Clone)]
pub struct PrometheusMetricsConfig {
  /// HTTP path where the Prometheus scrape endpoint will be exposed.
  pub endpoint_path: String,
  /// Latency histogram bucket boundaries (seconds). Defaults to
  /// [`DEFAULT_LATENCY_BUCKETS_SEC`].
  pub buckets: Vec<f64>,
}

#[cfg(feature = "metrics-prometheus")]
impl Default for PrometheusMetricsConfig {
  fn default() -> Self {
    Self {
      endpoint_path: "/metrics".to_string(),
      buckets: DEFAULT_LATENCY_BUCKETS_SEC.to_vec(),
    }
  }
}

#[cfg(feature = "metrics-prometheus")]
impl PrometheusMetricsConfig {
  /// Replaces the histogram bucket schedule.
  pub fn with_buckets(mut self, buckets: Vec<f64>) -> Self {
    self.buckets = buckets;
    self
  }

  /// Installs a Prometheus metrics backend and a scrape endpoint on the router.
  pub fn install(self, router: &mut Router) -> Arc<Registry> {
    let registry = Arc::new(Registry::new());
    let backend = prometheus_backend::PrometheusMetricsBackend::with_buckets(
      (*registry).clone(),
      self.buckets,
    );
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
  /// OTLP endpoint URL for metrics export.
  pub endpoint: String,
}

#[cfg(feature = "metrics-opentelemetry")]
impl Default for OtelMetricsConfig {
  fn default() -> Self {
    Self {
      meter_name: "tako",
      endpoint: "http://localhost:4318/v1/metrics".to_string(),
    }
  }
}

#[cfg(feature = "metrics-opentelemetry")]
impl OtelMetricsConfig {
  /// Sets the OTLP endpoint URL.
  pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
    self.endpoint = endpoint.into();
    self
  }

  /// Sets the meter name.
  pub fn with_meter_name(mut self, name: &'static str) -> Self {
    self.meter_name = name;
    self
  }

  /// Installs an OpenTelemetry metrics backend with OTLP exporter.
  ///
  /// Returns the `SdkMeterProvider` which should be kept alive for the
  /// application lifetime. Call `shutdown()` on it during graceful shutdown.
  pub fn install(
    self,
    router: &mut Router,
  ) -> Result<opentelemetry_sdk::metrics::SdkMeterProvider> {
    use opentelemetry::global;
    use opentelemetry_otlp::WithExportConfig;

    let exporter = opentelemetry_otlp::MetricExporter::builder()
      .with_http()
      .with_endpoint(&self.endpoint)
      .build()
      .map_err(|e| anyhow::anyhow!("failed to create OTLP metric exporter: {}", e))?;

    let meter_provider = opentelemetry_sdk::metrics::SdkMeterProvider::builder()
      .with_periodic_exporter(exporter)
      .build();

    global::set_meter_provider(meter_provider.clone());

    let meter = global::meter(self.meter_name);
    let backend = opentelemetry_backend::OtelMetricsBackend::new(meter);
    let plugin = MetricsPlugin::new(backend);

    router.plugin(plugin);

    Ok(meter_provider)
  }
}
