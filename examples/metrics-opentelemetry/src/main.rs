//! Metrics with OpenTelemetry OTLP exporter example.
//!
//! This example demonstrates how to use Tako's metrics plugin with
//! OpenTelemetry OTLP exporter to send metrics to collectors like
//! Prometheus, Jaeger, or the OpenTelemetry Collector.
//!
//! Run this example with:
//! ```sh
//! cargo run --example metrics-opentelemetry --features metrics-opentelemetry
//! ```
//!
//! To test with Prometheus:
//! ```sh
//! docker run -p 9090:9090 prom/prometheus --web.enable-otlp-receiver
//! ```
//! Then update the endpoint to "http://localhost:9090/api/v1/otlp/v1/metrics"

use anyhow::Result;
use tako::Method;
use tako::plugins::metrics::OtelMetricsConfig;
use tako::responder::Responder;
use tako::router::Router;
use tokio::net::TcpListener;

async fn hello() -> impl Responder {
  "Hello from metrics example".into_response()
}

async fn health() -> impl Responder {
  "OK".into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", hello);
  router.route(Method::GET, "/health", health);

  // Install the OpenTelemetry metrics plugin with OTLP exporter.
  // By default, metrics are exported to http://localhost:4318/v1/metrics
  let meter_provider = OtelMetricsConfig::default()
    .with_endpoint("http://localhost:4318/v1/metrics")
    .install(&mut router)?;

  println!("Server running on http://127.0.0.1:8080");
  println!("Metrics being exported via OTLP to http://localhost:4318/v1/metrics");

  tako::serve(listener, router).await;

  // Shutdown the meter provider to flush remaining metrics
  meter_provider.shutdown()?;

  Ok(())
}
