//! TLS HTTP server on Tokio with rustls; ALPN offers `h2` when the `http2` feature is on.

mod entry;
mod serve;

pub use entry::serve_tls_with_rustls_config;
pub use entry::serve_tls_with_rustls_config_and_shutdown;
pub use serve::run_with_config;
