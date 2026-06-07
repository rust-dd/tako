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

  // Install default crypto provider for rustls (required for QUIC/TLS).
  // Use `aws_lc_rs` to match the TLS path (`builder.rs`); installing two
  // different providers in the same process was order-dependent and the
  // loser silently dropped its `Err`, leaving connections to fail later.
  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }

  let certs_vec = load_certs(certs.unwrap_or("cert.pem"))?;
  let key = load_key(key.unwrap_or("key.pem"))?;

  let mut tls_config = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(certs_vec, key)?;

  // 0-RTT (early data) is disabled by default: the server has no replay-protection
  // wiring on the request path, so accepting early-data application bytes would
  // expose idempotent endpoints to replay attacks. Re-enabling requires plumbing a
  // replay cache and a typed extractor — see V2_ROADMAP.md § 1.5.
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
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  // Install default crypto provider for rustls (required for QUIC/TLS).
  // Use `aws_lc_rs` to match the TLS path (`builder.rs`); installing two
  // different providers in the same process was order-dependent and the
  // loser silently dropped its `Err`, leaving connections to fail later.
  if rustls::crypto::CryptoProvider::get_default().is_none() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  }

  // Defensively disable 0-RTT before handing the config to QuicServerConfig:
  // the PEM-loaded `run` path above already clears `max_early_data_size`, but
  // every caller-supplied (`TlsCert::Der`, `Resolver`, mTLS) config reaches
  // this function unmodified, and rustls' default permits 0-RTT — which Tako
  // has no replay-protection wiring for. Without this clear an attacker could
  // replay captured early-data application bytes to any idempotent endpoint.
  // Clone-on-mutate: `tls_config: Arc<...>` is immutable, but we always need
  // a fresh copy for `QuicServerConfig::try_from` below anyway.
  let mut tls_config_inner = (*tls_config).clone();
  tls_config_inner.max_early_data_size = 0;

  // QuicServerConfig wraps a rustls::ServerConfig; it requires the underlying
  // config to set ALPN to h3. Calling `try_from` errors otherwise, so we trust
  // the caller (or the build_rustls_server_config helper) to pre-set ALPN.
  let mut server_config =
    quinn::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls_config_inner)?));
  server_config.transport_config(Arc::new(transport_config_from(&config)));

  let socket_addr: SocketAddr = addr.parse()?;
  let endpoint = quinn::Endpoint::server(server_config, socket_addr)?;

  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

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

  // Per-connection graceful shutdown signal. `CancellationToken` is sticky:
  // once cancelled, all subsequent and pre-existing `.cancelled()` awaits
  // resolve, so a connection that handshakes AFTER the outer shutdown signal
  // still observes the GOAWAY trigger (this closes the H3 GOAWAY race that
  // `Notify::notify_waiters` previously had — it only woke already-registered
  // waiters, leaving late-handshaking connections hard-closing instead of
  // draining gracefully).
  let conn_shutdown = tokio_util::sync::CancellationToken::new();

  let cancel = tokio_util::sync::CancellationToken::new();
  if let Some(s) = signal {
    let cancel_for_signal = cancel.clone();
    tokio::spawn(async move {
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
      }
      () = cancel.cancelled() => {
        tracing::info!("Shutdown signal received, sending HTTP/3 GOAWAY...");
        break;
      }
    }
  }

  // Phase 1: trigger the CancellationToken so every spawned connection task
  // (including those that began their handshake just before this point) sees
  // shutdown and emits a GOAWAY frame.
  conn_shutdown.cancel();

  // Phase 2: wait for in-flight connections to finish gracefully, bounded by
  // the global drain deadline.
  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining HTTP/3 connections",
      drain_timeout,
      join_set.len()
    );
    join_set.abort_all();
  }

  // Phase 3: close the endpoint after grace expired (or all conns settled).
  endpoint.close(0u32.into(), b"server shutting down");
  endpoint.wait_idle().await;
  tracing::info!("HTTP/3 server shut down gracefully");
  Ok(())
}
