#![cfg(feature = "tls")]
#![cfg_attr(docsrs, doc(cfg(feature = "tls")))]

//! TLS-enabled HTTP server implementation for secure connections (compio runtime).
//!
//! HTTP/2 over ALPN `h2` runs through the `send_wrapper` glue in
//! `crate::compio_h2`; its module docs spell out the soundness contract.

mod accept;
mod serve;

pub use accept::run_with_config;
pub use serve::load_certs;
pub use serve::load_key;
pub use serve::run;
#[allow(deprecated)]
pub use serve::serve_tls;
#[allow(deprecated)]
pub use serve::serve_tls_with_config;
pub use serve::serve_tls_with_rustls_config;
pub use serve::serve_tls_with_rustls_config_and_shutdown;
#[allow(deprecated)]
pub use serve::serve_tls_with_shutdown;
#[allow(deprecated)]
pub use serve::serve_tls_with_shutdown_and_config;
