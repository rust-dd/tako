//! Public compio TLS entry points and rustls `ServerConfig` construction.
//!
//! These functions build (or accept) the shared `Arc<rustls::ServerConfig>`,
//! wire ALPN to the enabled HTTP protocols, and hand off to the accept loop.

use std::future::Future;
use std::sync::Arc;

use compio::net::TcpListener;
use rustls::ServerConfig as RustlsServerConfig;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;

use super::accept::run_with_config;
use crate::ServerConfig;

/// Starts a TLS-enabled HTTP server with the given listener, router, and certificates.
pub async fn serve_tls(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
) {
  if let Err(e) = run(
    listener,
    router,
    certs,
    key,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Starts a TLS-enabled HTTP server (compio) with graceful shutdown support.
pub async fn serve_tls_with_shutdown(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(
    listener,
    router,
    certs,
    key,
    Some(signal),
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls`] with caller-supplied [`ServerConfig`].
pub async fn serve_tls_with_config(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  config: ServerConfig,
) {
  if let Err(e) = run(
    listener,
    router,
    certs,
    key,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls_with_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_tls_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, certs, key, Some(signal), config).await {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls`] with a caller-built `Arc<rustls::ServerConfig>` (compio).
pub async fn serve_tls_with_rustls_config(
  listener: TcpListener,
  router: Router,
  rustls_config: Arc<RustlsServerConfig>,
  config: ServerConfig,
) {
  if let Err(e) = run_with_config(
    listener,
    router,
    rustls_config,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("TLS server error: {e}");
  }
}

/// Like [`serve_tls_with_rustls_config`] with graceful shutdown.
pub async fn serve_tls_with_rustls_config_and_shutdown(
  listener: TcpListener,
  router: Router,
  rustls_config: Arc<RustlsServerConfig>,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run_with_config(listener, router, rustls_config, Some(signal), config).await {
    tracing::error!("TLS server error: {e}");
  }
}

/// Runs the TLS server loop, handling secure connections and request dispatch.
pub async fn run(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let certs = load_certs(certs.unwrap_or("cert.pem"))?;
  let key = load_key(key.unwrap_or("key.pem"))?;

  let mut tls_config = RustlsServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs, key)?;

  #[cfg(feature = "http2")]
  {
    tls_config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
  }

  #[cfg(not(feature = "http2"))]
  {
    tls_config.alpn_protocols = vec![b"http/1.1".to_vec()];
  }

  run_with_config(listener, router, Arc::new(tls_config), signal, config).await
}

/// Loads TLS certificates from a PEM-encoded file. Re-export of
/// [`tako_rs_core::tls::load_certs`].
pub use tako_rs_core::tls::load_certs;
/// Loads a private key from a PEM-encoded file. Accepts PKCS#8, PKCS#1 (RSA),
/// and SEC1 (EC) PEM blocks. Re-export of [`tako_rs_core::tls::load_key`].
pub use tako_rs_core::tls::load_key;
