//! HTTP/3 over QUIC on Tokio, using `h3` with `quinn` as the transport.

mod connection;
pub(crate) mod run;
mod serve;

pub use serve::serve_h3_with_rustls_config;
pub use serve::serve_h3_with_rustls_config_and_shutdown;
