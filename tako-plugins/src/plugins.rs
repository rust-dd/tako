//! Built-in plugin implementations.
//!
//! Each submodule provides one ready-to-use plugin (CORS, compression, rate
//! limiting, idempotency, metrics) gated behind the appropriate feature flag.

/// Compression plugin for automatic response compression.
#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod compression;

/// CORS (Cross-Origin Resource Sharing) plugin for handling cross-origin requests.
#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod cors;

/// Rate limiting plugin for controlling request frequency.
#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod rate_limiter;

/// Metrics/tracing plugin for integrating with systems like Prometheus or OpenTelemetry.
#[cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry"))]
#[cfg_attr(
  docsrs,
  doc(cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry")))
)]
pub mod metrics;

/// Idempotency-Key based request de-duplication plugin.
#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod idempotency;
