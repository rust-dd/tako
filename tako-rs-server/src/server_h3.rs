#![cfg(feature = "http3")]
#![cfg_attr(docsrs, doc(cfg(feature = "http3")))]

//! HTTP/3 server implementation using QUIC transport.
//!
//! This module provides HTTP/3 support for Tako web servers using the h3 crate
//! with Quinn as the QUIC transport. HTTP/3 offers improved performance over
//! HTTP/1.1 and HTTP/2 through features like reduced latency, better multiplexing,
//! and built-in encryption via QUIC.
//!
//! # Examples
//!
//! ```rust,no_run
//! # #[cfg(feature = "http3")]
//! use tako::{serve_h3, router::Router, Method, responder::Responder, types::Request};
//!
//! # #[cfg(feature = "http3")]
//! async fn hello(_: Request) -> impl Responder {
//!     "Hello, HTTP/3 World!".into_response()
//! }
//!
//! # #[cfg(feature = "http3")]
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! serve_h3(router, "[::]:4433", Some("cert.pem"), Some("key.pem")).await;
//! # Ok(())
//! # }
//! ```

mod config;
mod connection;
mod request;
mod run;
mod serve;

pub use serve::serve_h3;
pub use serve::serve_h3_with_config;
pub use serve::serve_h3_with_rustls_config;
pub use serve::serve_h3_with_rustls_config_and_shutdown;
pub use serve::serve_h3_with_shutdown;
pub use serve::serve_h3_with_shutdown_and_config;
/// Loads TLS certificates from a PEM-encoded file. Re-export of
/// [`tako_rs_core::tls::load_certs`].
pub use tako_rs_core::tls::load_certs;
/// Loads a private key from a PEM-encoded file. Accepts PKCS#8, PKCS#1 (RSA),
/// and SEC1 (EC) PEM blocks. Re-export of [`tako_rs_core::tls::load_key`].
pub use tako_rs_core::tls::load_key;
