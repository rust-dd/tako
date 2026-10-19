use std::future::Future;
use std::sync::Arc;

use tako_rs_core::router::Router;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

use super::run_with_config;
use crate::ServerConfig;

/// Serves TLS HTTP with a prebuilt rustls config, e.g. from [`crate::build_rustls_server_config`]
/// for the `Der`, `Resolver` and mTLS [`crate::TlsCert`] variants; errors are logged, not returned.
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
