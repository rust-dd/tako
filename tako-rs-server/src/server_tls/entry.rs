use std::future::Future;
use std::sync::Arc;

use tako_rs_core::router::Router;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

use super::run;
use super::run_with_config;
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

/// Starts a TLS-enabled HTTP server with graceful shutdown support.
pub async fn serve_tls_with_shutdown(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()> + Send + 'static,
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

/// Like [`serve_tls`] but with caller-supplied [`ServerConfig`].
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

/// Like [`serve_tls_with_shutdown`] but with caller-supplied [`ServerConfig`].
pub async fn serve_tls_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, certs, key, Some(signal), config).await {
    tracing::error!("TLS server error: {e}");
  }
}

/// Run with a caller-built `Arc<rustls::ServerConfig>`. Use this when
/// constructing the TLS config via [`crate::TlsCert`] variants beyond
/// `PemPaths` (`Der`, `Resolver`, mTLS) — the [`crate::build_rustls_server_config`]
/// helper handles the assembly.
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
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run_with_config(listener, router, rustls_config, Some(signal), config).await {
    tracing::error!("TLS server error: {e}");
  }
}
