use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;

use quinn::crypto::rustls::QuicServerConfig;
use tako_rs_core::router::Router;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;

use super::config::transport_config_from;
use super::connection::handle_connection;
use super::load_certs;
use super::load_key;
use crate::ServerConfig;

/// Runs the HTTP/3 server loop.
pub(crate) async fn run(
  router: Router,
  addr: &str,
  certs: Option<&str>,
  key: Option<&str>,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }

  let certs_vec = load_certs(certs.unwrap_or("cert.pem"))?;
  let key = load_key(key.unwrap_or("key.pem"))?;

  let mut tls_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs_vec, key)?;

  // Early data requires replay protection, which this server does not provide.
  tls_config.max_early_data_size = 0;
  tls_config.alpn_protocols = vec![b"h3".to_vec()];

  run_with_rustls_config(router, addr, Arc::new(tls_config), signal, config).await
}

/// Variant of [`run`] that accepts a pre-built `Arc<rustls::ServerConfig>`.
pub(crate) async fn run_with_rustls_config(
  router: Router,
  addr: &str,
  tls_config: Arc<rustls::ServerConfig>,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let endpoint = bind_endpoint(addr, tls_config, &config)?;
  run_endpoint(endpoint, router, signal, config).await
}

pub(crate) fn bind_endpoint(
  addr: &str,
  tls_config: Arc<rustls::ServerConfig>,
  config: &ServerConfig,
) -> Result<quinn::Endpoint, BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }

  // Early data requires replay protection, which this server does not provide.
  let mut tls_config_inner = (*tls_config).clone();
  tls_config_inner.max_early_data_size = 0;

  let mut server_config =
    quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config_inner)?));
  server_config.transport_config(Arc::new(transport_config_from(config)));

  let socket_addr: SocketAddr = addr.parse()?;
  quinn::Endpoint::server(server_config, socket_addr).map_err(Into::into)
}

pub(crate) async fn run_endpoint(
  endpoint: quinn::Endpoint,
  router: Router,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  let addr_str = endpoint.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "quic", true).await;

  tracing::info!("Tako HTTP/3 listening on {}", addr_str);

  let mut join_set = tokio::task::JoinSet::new();
  let drain_timeout = config.drain_timeout;
  let goaway_grace = config.h3_goaway_grace.min(drain_timeout);
  let h3_use_retry = config.h3_use_retry;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

  let conn_shutdown = tokio_util::sync::CancellationToken::new();

  let cancel = tokio_util::sync::CancellationToken::new();
  let mut signal_tasks = tokio::task::JoinSet::new();
  if let Some(s) = signal {
    let cancel_for_signal = cancel.clone();
    signal_tasks.spawn(async move {
      s.await;
      cancel_for_signal.cancel();
    });
  }

  loop {
    tokio::select! {
      maybe_conn = endpoint.accept() => {
        let Some(incoming) = maybe_conn else { break };

        // Optional address-validation retry. Defends against UDP source-IP
        // spoofing amplification by forcing the client through one extra
        // round-trip with a server-issued retry token.
        if h3_use_retry && !incoming.remote_address_validated() {
          if let Err(e) = incoming.retry() {
            tracing::debug!("HTTP/3 retry refused: {e}");
          }
          continue;
        }

        let permit = if let Some(sem) = &max_conn_semaphore {
          tokio::select! {
            biased;
            () = cancel.cancelled() => break,
            permit = sem.clone().acquire_owned() => match permit {
              Ok(p) => Some(p),
              Err(_) => continue,
            },
          }
        } else {
          None
        };
        let router = router.clone();
        let conn_shutdown = conn_shutdown.clone();

        join_set.spawn(async move {
          match incoming.await {
            Ok(conn) => {
              let remote_addr = conn.remote_address();

              #[cfg(feature = "signals")]
              signal_tx::emit_connection_opened(&remote_addr.to_string(), true, Some("h3")).await;

              if let Err(e) =
                handle_connection(conn, router, remote_addr, conn_shutdown, goaway_grace).await
              {
                tracing::error!("HTTP/3 connection error: {e}");
              }

              #[cfg(feature = "signals")]
              signal_tx::emit_connection_closed(&remote_addr.to_string(), true, Some("h3")).await;
            }
            Err(e) => {
              tracing::error!("QUIC connection failed: {e}");
            }
          }

          drop(permit);
        });
        while join_set.try_join_next().is_some() {}
      }
      () = cancel.cancelled() => {
        tracing::info!("Shutdown signal received, sending HTTP/3 GOAWAY...");
        break;
      }
    }
  }

  conn_shutdown.cancel();
  let deadline = tokio::time::Instant::now() + drain_timeout;
  let drain = tokio::time::timeout_at(deadline, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining HTTP/3 connections",
      drain_timeout,
      join_set.len()
    );
    join_set.shutdown().await;
  }

  endpoint.close(0u32.into(), b"server shutting down");
  let _ = tokio::time::timeout_at(deadline, endpoint.wait_idle()).await;
  signal_tasks.shutdown().await;
  tracing::info!("HTTP/3 server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&addr_str, "quic", true).await;
  Ok(())
}
