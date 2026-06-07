//! OpenTelemetry metrics backend and OTLP exporter configuration.

use anyhow::Result;
use tako_rs_core::router::Router;

use crate::plugins::metrics::MetricsPlugin;

/// OpenTelemetry backend implementation.
#[cfg(feature = "metrics-opentelemetry")]
pub mod opentelemetry_backend {
  use opentelemetry::KeyValue;
  use opentelemetry::metrics::Counter;
  use opentelemetry::metrics::Meter;
  use tako_rs_core::signals::Signal;

  use crate::plugins::metrics::MetricsBackend;

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
    // Bound label cardinality: a matched route template is finite; the raw
    // URI path is attacker-controlled and would explode label space.
    signal
      .metadata
      .get("route")
      .cloned()
      .unwrap_or_else(|| "unmatched".to_string())
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
      .map_err(|e| anyhow::anyhow!("failed to create OTLP metric exporter: {e}"))?;

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
