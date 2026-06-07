use std::future::Future;
use std::sync::Arc;

use tako_rs_core::router::Router;

use super::run::run;
use super::run::run_with_rustls_config;
use crate::ServerConfig;

/// Starts an HTTP/3 server with the given router and certificates.
///
/// This function creates a QUIC endpoint and listens for incoming HTTP/3 connections.
/// Unlike TCP-based servers, HTTP/3 uses UDP and QUIC for transport.
///
/// # Arguments
///
/// * `router` - The Tako router containing route definitions
/// * `addr` - The socket address to bind to (e.g., `"[::]:4433"` for IPv6)
/// * `certs` - Optional path to the TLS certificate file (defaults to "cert.pem")
/// * `key` - Optional path to the TLS private key file (defaults to "key.pem")
pub async fn serve_h3(router: Router, addr: &str, certs: Option<&str>, key: Option<&str>) {
  if let Err(e) = run(
    router,
    addr,
    certs,
    key,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Starts an HTTP/3 server with graceful shutdown support.
pub async fn serve_h3_with_shutdown(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()> + Send + 'static,
) {
  if let Err(e) = run(
    router,
    addr,
    certs,
    key,
    Some(signal),
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Like [`serve_h3`] with caller-supplied [`ServerConfig`].
pub async fn serve_h3_with_config(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  config: ServerConfig,
) {
  if let Err(e) = run(
    router,
    addr,
    certs,
    key,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Like [`serve_h3_with_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_h3_with_shutdown_and_config(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run(router, addr, certs, key, Some(signal), config).await {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Run an HTTP/3 server with a caller-built `Arc<rustls::ServerConfig>`. The
/// caller is responsible for setting `alpn_protocols = [b"h3"]` (the
/// [`crate::build_rustls_server_config`] helper does this when given the right
/// ALPN list). Use this when constructing the TLS config via [`crate::TlsCert`]
/// variants beyond `PemPaths`.
pub async fn serve_h3_with_rustls_config(
  router: Router,
  addr: &str,
  rustls_config: Arc<rustls::ServerConfig>,
  config: ServerConfig,
) {
  if let Err(e) = run_with_rustls_config(
    router,
    addr,
    rustls_config,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("HTTP/3 server error: {e}");
  }
}

/// Like [`serve_h3_with_rustls_config`] with graceful shutdown.
pub async fn serve_h3_with_rustls_config_and_shutdown(
  router: Router,
  addr: &str,
  rustls_config: Arc<rustls::ServerConfig>,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run_with_rustls_config(router, addr, rustls_config, Some(signal), config).await {
    tracing::error!("HTTP/3 server error: {e}");
  }
}
