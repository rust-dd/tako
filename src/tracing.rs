//! Distributed tracing integration for observability and debugging.
//!
//! This module provides tracing setup and configuration for Tako applications using the
//! `tracing` ecosystem. It configures structured logging with file names, line numbers,
//! log levels, and span events. The tracing system helps with debugging, monitoring,
//! and understanding application behavior in development and production environments.

use tracing_subscriber::{
  Layer, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

pub use tracing::level_filters::LevelFilter;

pub static mut TRACING_LEVEL: LevelFilter = LevelFilter::DEBUG;

pub fn set_tracing_level(level_filter: LevelFilter) {
  unsafe { TRACING_LEVEL = level_filter };
}

/// Initializes the global tracing subscriber with formatted output.
pub fn init_tracing() {
  tracing_subscriber::registry()
    .with(
      tracing_subscriber::fmt::layer()
        .with_span_events(FmtSpan::CLOSE)
        .with_file(true)
        .with_line_number(true)
        .with_level(true)
        .with_filter(unsafe { TRACING_LEVEL }),
    )
    .init();
}
