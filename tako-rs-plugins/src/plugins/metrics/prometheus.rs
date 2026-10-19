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

pub mod prometheus_backend {
  use std::sync::Arc;

  use prometheus::HistogramOpts;
  use prometheus::HistogramVec;
  use prometheus::IntCounterVec;
  use prometheus::Opts;
  use prometheus::Registry;
  use prometheus::core::Collector;
  use tako_rs_core::signals::Signal;

  use super::super::recorder::route_label;
  use super::super::recorder::transport_label;
  use crate::plugins::metrics::DEFAULT_LATENCY_BUCKETS_SEC;
  use crate::plugins::metrics::MetricsBackend;

  /// `AlreadyReg` only warns, so a second install keeps the first metrics; other errors are
  /// misconfiguration and panic.
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

    /// Panics if registration fails for any reason other than the metric already existing.
    pub fn with_buckets(registry: Registry, buckets: Vec<f64>) -> Self {
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
      // `duration_us` is present only when latency tracking is on; Prometheus wants seconds.
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

#[derive(Clone)]
pub struct PrometheusMetricsConfig {
  /// Default: `/metrics`.
  pub endpoint_path: String,
  /// Latency histogram bucket boundaries (seconds). Defaults to
  /// [`DEFAULT_LATENCY_BUCKETS_SEC`].
  pub buckets: Vec<f64>,
}

impl Default for PrometheusMetricsConfig {
  fn default() -> Self {
    Self {
      endpoint_path: "/metrics".to_string(),
      buckets: DEFAULT_LATENCY_BUCKETS_SEC.to_vec(),
    }
  }
}

impl PrometheusMetricsConfig {
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
    router.with_state(registry.clone());

    let path = self.endpoint_path;
    router.route(Method::GET, &path, prometheus_metrics_handler);

    registry
  }
}

async fn prometheus_metrics_handler(State(registry): State<Arc<Registry>>) -> impl Responder {
  let encoder = TextEncoder::new();
  let metric_families = registry.gather();

  let mut buf = Vec::new();
  if let Err(e) = encoder.encode(&metric_families, &mut buf) {
    // A 5xx lets scrapers alert; they treat any 2xx as success whatever the body says.
    tracing::error!("prometheus encode failed: {e}");
    return (
      http::StatusCode::INTERNAL_SERVER_ERROR,
      format!("failed to encode metrics: {e}"),
    )
      .into_response();
  }

  // The text format is ASCII, so this is an encoder bug; an empty 200 would read as no metrics.
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
