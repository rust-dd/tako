//! Compio TLS entry points that build or take a rustls config and hand it to the accept loop.

use std::future::Future;
use std::sync::Arc;

use compio::net::TcpListener;
use rustls::ServerConfig as RustlsServerConfig;
use tako_rs_core::router::Router;

use super::accept::run_with_config;
use crate::ServerConfig;

/// Serves TLS HTTP with a prebuilt rustls config, e.g. from [`crate::build_rustls_server_config`];
/// errors are logged, not returned.
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
