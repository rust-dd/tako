//! Feeds Tako signals into a pluggable metrics backend; the `metrics-prometheus` and
//! `metrics-opentelemetry` features each provide one.

mod recorder;

pub use recorder::DEFAULT_LATENCY_BUCKETS_SEC;
pub use recorder::MetricsBackend;
pub use recorder::MetricsPlugin;

#[cfg(feature = "metrics-prometheus")]
mod prometheus;

#[cfg(feature = "metrics-prometheus")]
pub use prometheus::PrometheusMetricsConfig;
#[cfg(feature = "metrics-prometheus")]
pub use prometheus::prometheus_backend;

#[cfg(feature = "metrics-opentelemetry")]
mod opentelemetry;

#[cfg(feature = "metrics-opentelemetry")]
pub use opentelemetry::OtelMetricsConfig;
#[cfg(feature = "metrics-opentelemetry")]
pub use opentelemetry::opentelemetry_backend;
