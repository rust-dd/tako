//! One plugin per module, built on `tako_rs_core::plugins::TakoPlugin`.

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod compression;

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod cors;

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod rate_limiter;

#[cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry"))]
#[cfg_attr(
  docsrs,
  doc(cfg(any(feature = "metrics-prometheus", feature = "metrics-opentelemetry")))
)]
pub mod metrics;

#[cfg(feature = "plugins")]
#[cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
pub mod idempotency;
