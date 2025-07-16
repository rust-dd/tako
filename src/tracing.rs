//! Distributed tracing integration for observability and debugging.
//!
//! This module provides tracing setup and configuration for Tako applications using the
//! `tracing` ecosystem. It configures structured logging with file names, line numbers,
//! log levels, and span events. The tracing system helps with debugging, monitoring,
//! and understanding application behavior in development and production environments.
//!
//! # Examples
//!
//! ```rust
//! # #[cfg(feature = "tako-tracing")]
//! use tako::tracing::init_tracing;
//!
//! # #[cfg(feature = "tako-tracing")]
//! # fn example() {
//! // Initialize tracing for the application
//! init_tracing();
//!
//! // Now you can use tracing macros throughout your application
//! tracing::info!("Application started");
//! tracing::debug!("Debug information");
//! # }
//! ```

use tracing::level_filters::LevelFilter;
use tracing_subscriber::{
    Layer, fmt::format::FmtSpan, layer::SubscriberExt, util::SubscriberInitExt,
};

/// Initializes the global tracing subscriber with formatted output.
///
/// Sets up a comprehensive tracing configuration that includes file names, line numbers,
/// log levels, and span close events. The subscriber is configured with DEBUG level
/// filtering and formatted output suitable for development and debugging. This function
/// should be called once at application startup.
///
/// # Examples
///
/// ```rust
/// # #[cfg(feature = "tako-tracing")]
/// use tako::tracing::init_tracing;
///
/// # #[cfg(feature = "tako-tracing")]
/// # fn example() {
/// // Initialize tracing at application startup
/// init_tracing();
///
/// // Use tracing macros in your application
/// tracing::info!("Server starting");
/// tracing::debug!("Configuration loaded");
/// tracing::warn!("Deprecated feature used");
/// tracing::error!("Failed to connect to database");
/// # }
/// ```
pub fn init_tracing() {
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .with_span_events(FmtSpan::CLOSE)
                .with_file(true)
                .with_line_number(true)
                .with_level(true)
                .with_filter(LevelFilter::DEBUG),
        )
        .init();
}
