//! TLS HTTP server on compio; HTTP/2 over ALPN `h2` runs on the `send_wrapper` glue in `compio_h2`.

mod accept;
mod serve;

pub use accept::run_with_config;
pub use serve::serve_tls_with_rustls_config;
pub use serve::serve_tls_with_rustls_config_and_shutdown;
