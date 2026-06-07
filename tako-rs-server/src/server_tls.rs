#![cfg(feature = "tls")]
#![cfg_attr(docsrs, doc(cfg(feature = "tls")))]

//! TLS-enabled HTTP server implementation for secure connections.
//!
//! This module provides TLS/SSL support for Tako web servers using rustls for encryption.
//! It handles secure connection establishment, certificate loading, and supports both
//! HTTP/1.1 and HTTP/2 protocols (when the http2 feature is enabled). The main entry
//! point is `serve_tls` which starts a secure server with the provided certificates.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "tls")]
//! use tako::{serve_tls, router::Router, Method, responder::Responder, types::Request};
//! # #[cfg(feature = "tls")]
//! use tokio::net::TcpListener;
//!
//! # #[cfg(feature = "tls")]
//! async fn hello(_: Request) -> impl Responder {
//!     "Hello, Secure World!".into_response()
//! }
//!
//! # #[cfg(feature = "tls")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("127.0.0.1:8443").await?;
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! serve_tls(listener, router, Some("cert.pem"), Some("key.pem")).await;
//! # Ok(())
//! # }
//! ```

// HTTP/2 hardening + connection lifetimes are sourced from `ServerConfig`,
// whose `Default` mirrors the historical hardcoded values (30 s drain, 100
// streams, 16 KiB header list, 1 MiB send buf, 50 pending-accept resets).
//
// Pass a custom [`ServerConfig`] via [`serve_tls_with_config`] /
// [`serve_tls_with_shutdown_and_config`] to override individual knobs while
// keeping perf-neutral defaults for everything you don't touch.

mod config;
mod entry;
mod serve;

pub use config::run;
pub use entry::serve_tls;
pub use entry::serve_tls_with_config;
pub use entry::serve_tls_with_rustls_config;
pub use entry::serve_tls_with_rustls_config_and_shutdown;
pub use entry::serve_tls_with_shutdown;
pub use entry::serve_tls_with_shutdown_and_config;
pub use serve::run_with_config;
/// Loads TLS certificates from a PEM-encoded file.
///
/// Thin re-export of [`tako_rs_core::tls::load_certs`]; preserved for backward
/// compatibility.
pub use tako_rs_core::tls::load_certs;
/// Loads a private key from a PEM-encoded file.
///
/// Accepts PKCS#8, PKCS#1 (RSA), and SEC1 (EC) PEM blocks. Thin re-export of
/// [`tako_rs_core::tls::load_key`]; preserved for backward compatibility.
pub use tako_rs_core::tls::load_key;
