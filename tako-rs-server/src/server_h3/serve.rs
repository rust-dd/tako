use std::future::Future;
use std::sync::Arc;

use tako_rs_core::router::Router;

use super::run::run_with_rustls_config;
use crate::ServerConfig;

/// Serves HTTP/3 with a prebuilt rustls config, whose ALPN list must contain `h3` (see
/// [`crate::build_rustls_server_config`]); errors are logged, not returned.
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
