use anyhow::Result;
use tako_rs_core::router::Router;

use crate::plugins::metrics::MetricsPlugin;

pub mod opentelemetry_backend {
  use opentelemetry::KeyValue;
  use opentelemetry::metrics::Counter;
  use opentelemetry::metrics::Meter;
  use tako_rs_core::signals::Signal;

  use super::super::recorder::route_label;
  use super::super::recorder::transport_label;
  use crate::plugins::metrics::MetricsBackend;

  /// Counters only: unlike the Prometheus backend it records no latency histogram.
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

  impl MetricsBackend for OtelMetricsBackend {
    fn on_request_completed(&self, signal: &Signal) {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let route = route_label(signal).to_owned();
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
      let route = route_label(signal).to_owned();
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

#[derive(Clone)]
pub struct OtelMetricsConfig {
  /// Default: `tako`.
  pub meter_name: &'static str,
  /// OTLP/HTTP metrics endpoint; default `http://localhost:4318/v1/metrics`.
  pub endpoint: String,
}

impl Default for OtelMetricsConfig {
  fn default() -> Self {
    Self {
      meter_name: "tako",
      endpoint: "http://localhost:4318/v1/metrics".to_string(),
    }
  }
}

impl OtelMetricsConfig {
  pub fn with_endpoint(mut self, endpoint: impl Into<String>) -> Self {
    self.endpoint = endpoint.into();
    self
  }

  pub fn with_meter_name(mut self, name: &'static str) -> Self {
    self.meter_name = name;
    self
  }

  /// Also sets the global meter provider. Keep the returned provider alive and call its
  /// `shutdown()` during graceful shutdown.
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
