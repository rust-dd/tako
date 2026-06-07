//! Prometheus metrics backend, scrape-endpoint configuration, and handler.

use std::sync::Arc;

use prometheus::Encoder;
use prometheus::Registry;
use prometheus::TextEncoder;
use tako_rs_core::Method;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_extractors::state::State;

use crate::plugins::metrics::DEFAULT_LATENCY_BUCKETS_SEC;
use crate::plugins::metrics::MetricsPlugin;

/// Prometheus backend implementation.
#[cfg(feature = "metrics-prometheus")]
pub mod prometheus_backend {
  use std::sync::Arc;

  use prometheus::HistogramOpts;
  use prometheus::HistogramVec;
  use prometheus::IntCounterVec;
  use prometheus::Opts;
  use prometheus::Registry;
  use prometheus::core::Collector;
  use tako_rs_core::signals::Signal;

  use crate::plugins::metrics::DEFAULT_LATENCY_BUCKETS_SEC;
  use crate::plugins::metrics::MetricsBackend;

  /// Register `collector` into `registry`. `AlreadyReg` is logged + ignored
  /// (idempotent install) so a double-install does not crash the server;
  /// other errors panic since they indicate a real misconfiguration.
  fn register_metric<C: Collector + Clone + 'static>(
    registry: &Registry,
    collector: &C,
    name: &str,
  ) {
    match registry.register(Box::new(collector.clone())) {
      Ok(()) => {}
      Err(prometheus::Error::AlreadyReg) => {
        tracing::warn!(
          metric = name,
          "PrometheusMetricsPlugin: metric already registered in this Registry — \
           ignoring second install (use a single shared plugin instance instead)"
        );
      }
      Err(e) => panic!("failed to register {name}: {e}"),
    }
  }

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

  /// Use the matched route template (`route`) when present; fall back to a
  /// fixed `"unmatched"` literal otherwise. Using the raw URI path here would
  /// let any client (e.g. a 404-scanner) produce unbounded distinct label
  /// values and blow up Prometheus memory.
  fn route_label(signal: &Signal) -> &str {
    signal
      .metadata
      .get("route")
      .map_or("unmatched", String::as_str)
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
    ///
    /// # Panics
    ///
    /// Panics only on impossible-by-construction conditions: every metric is
    /// built from compile-time-constant `Opts` (deterministic name + label set
    /// known to satisfy prometheus's identifier rules), and registered against
    /// a freshly-passed `Registry` where a name collision can only occur if the
    /// caller has already registered a metric under the reserved `tako_*`
    /// namespace. We surface those as `.expect(...)` rather than `Result`
    /// because the call is part of one-shot server startup — fatal here is
    /// strictly better than masking misconfiguration.
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

      // PPL-12: `Registry::register` returns `Err(AlreadyReg)` if the same
      // metric name is already registered. The original code `.unwrap()`d
      // these, so any user who installed PrometheusMetricsPlugin twice on
      // the same Registry (e.g. router-level + route-level) crashed the
      // process on second install. Treat AlreadyReg as a non-fatal warning
      // — the metrics from the first install remain authoritative; the
      // current plugin instance's metric handles are orphaned from the
      // scrape but the server keeps running. Any other registration error
      // remains a hard panic since it would indicate a tako bug (name
      // collision with a non-`tako_*` registrant, malformed Opts, etc.).
      register_metric(&registry, &http_requests_total, "http_requests_total");
      register_metric(
        &registry,
        &http_route_requests_total,
        "http_route_requests_total",
      );
      register_metric(&registry, &http_request_duration, "http_request_duration");
      register_metric(
        &registry,
        &connections_opened_total,
        "connections_opened_total",
      );
      register_metric(
        &registry,
        &connections_closed_total,
        "connections_closed_total",
      );

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
      let method = signal.metadata.get("method").map_or("", String::as_str);
      let route = route_label(signal);
      let status = signal.metadata.get("status").map_or("", String::as_str);
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
      let method = signal.metadata.get("method").map_or("", String::as_str);
      let route = route_label(signal);
      let status = signal.metadata.get("status").map_or("", String::as_str);
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
    let backend =
      prometheus_backend::PrometheusMetricsBackend::with_buckets((*registry).clone(), self.buckets);
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
  if let Err(e) = encoder.encode(&metric_families, &mut buf) {
    // PPL-27: surface encode failures as a real 5xx so the scraper's
    // alerting can fire, instead of returning a 200 body containing the
    // word "failed". Prometheus scrapers treat 2xx as success regardless
    // of body, so a 200/body-fail combination was effectively invisible.
    tracing::error!("prometheus encode failed: {e}");
    return (
      http::StatusCode::INTERNAL_SERVER_ERROR,
      format!("failed to encode metrics: {e}"),
    )
      .into_response();
  }

  // Prometheus text format is ASCII-only by construction; a non-UTF-8
  // payload here means the encoder violated its contract. Surface that
  // as 500 instead of silently serving an empty 200 (which scrapers
  // would treat as 'all metrics absent', triggering false alerts).
  match String::from_utf8(buf) {
    Ok(s) => s.into_response(),
    Err(e) => {
      tracing::error!("prometheus encoder emitted non-UTF-8 bytes: {e}");
      (
        http::StatusCode::INTERNAL_SERVER_ERROR,
        "prometheus encoder emitted non-UTF-8 bytes",
      )
        .into_response()
    }
  }
}
