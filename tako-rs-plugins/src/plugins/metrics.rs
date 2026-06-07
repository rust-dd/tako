#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Metrics and tracing plugin for integrating Tako's signal system with
//! backends like Prometheus or OpenTelemetry.
//!
//! This plugin listens to application-level and route-level signals and
//! updates metrics using an injected backend implementation. When the
//! `metrics-prometheus` or `metrics-opentelemetry` features are enabled,
//! a concrete backend is provided based on the selected feature, while
//! the core plugin logic remains backend-agnostic.

mod recorder;

pub use recorder::DEFAULT_LATENCY_BUCKETS_SEC;
#[cfg(feature = "signals")]
pub use recorder::MetricsBackend;
#[cfg(feature = "signals")]
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
