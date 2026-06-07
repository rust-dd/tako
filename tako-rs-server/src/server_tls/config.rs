use std::future::Future;
use std::sync::Arc;

use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;
use tokio::net::TcpListener;
use tokio_rustls::rustls::ServerConfig as RustlsServerConfig;

use super::load_certs;
use super::load_key;
use super::run_with_config;
use crate::ServerConfig;

/// Runs the TLS server loop, handling secure connections and request dispatch.
pub async fn run(
  listener: TcpListener,
  router: Router,
  certs: Option<&str>,
  key: Option<&str>,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
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
