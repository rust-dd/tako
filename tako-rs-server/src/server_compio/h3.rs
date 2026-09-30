//! HTTP/3 over QUIC on the compio runtime, driven by `compio-quic`.

use std::future::Future;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use compio::quic::Connection;
use compio::quic::Endpoint;
use compio::quic::EndpointConfig;
use compio::quic::crypto::rustls::QuicServerConfig;
use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::future::Either;
use futures_util::stream::FuturesUnordered;
use tako_rs_core::router::Router;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio_util::sync::CancellationToken;

use crate::ServerConfig;
use crate::h3_common::config::transport_config_from;
use crate::h3_common::request::handle_request;

/// Binds a QUIC endpoint for HTTP/3 on `addr`.
///
/// Must run inside a compio runtime, which drives the endpoint.
pub(crate) fn bind_endpoint(
  addr: &str,
  tls_config: &rustls::ServerConfig,
  config: &ServerConfig,
) -> Result<Endpoint, BoxError> {
  // Early data requires replay protection, which this server does not provide.
  let mut tls = tls_config.clone();
  tls.max_early_data_size = 0;

  let mut server_config =
    compio::quic::ServerConfig::with_crypto(Arc::new(QuicServerConfig::try_from(tls)?));
  server_config.transport_config(Arc::new(transport_config_from(config)));

  let socket_addr: SocketAddr = addr.parse()?;
  let socket = compio::net::UdpSocket::from_std(std::net::UdpSocket::bind(socket_addr)?)?;
  Ok(Endpoint::new(
    socket,
    EndpointConfig::default(),
    Some(server_config),
    None,
  )?)
}

pub(crate) async fn run_endpoint(
  endpoint: Endpoint,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let router = Arc::new(router);
  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  let addr_str = endpoint.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "quic", true).await;

  tracing::info!("Tako HTTP/3 listening on {}", addr_str);

  let mut connections = FuturesUnordered::new();
  let drain_timeout = config.drain_timeout;
  let goaway_grace = config.h3_goaway_grace.min(drain_timeout);
  let h3_use_retry = config.h3_use_retry;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  let conn_shutdown = CancellationToken::new();

  let signal = signal.map(|s| Box::pin(s));
  let mut signal_fused = std::pin::pin!(async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  });

  loop {
    let accept = std::pin::pin!(endpoint.wait_incoming());
    match futures_util::future::select(accept, signal_fused.as_mut()).await {
      Either::Left((None, _)) => break,
      Either::Left((Some(incoming), _)) => {
        // Optional address-validation retry. Defends against UDP source-IP
        // spoofing amplification by forcing the client through one extra
        // round-trip with a server-issued retry token.
        if h3_use_retry && !incoming.remote_address_validated() {
          if let Err(e) = incoming.retry() {
            tracing::debug!("HTTP/3 retry refused: {e}");
          }
          continue;
        }

        let permit = if let Some(sem) = max_conn_semaphore.as_ref() {
          let acquire = std::pin::pin!(sem.clone().acquire_owned());
          match futures_util::future::select(acquire, signal_fused.as_mut()).await {
            Either::Left((Ok(p), _)) => Some(p),
            Either::Left((Err(_), _)) => continue,
            Either::Right(_) => break,
          }
        } else {
          None
        };
        let router = router.clone();
        let conn_shutdown = conn_shutdown.clone();

        connections.push(compio::runtime::spawn(async move {
          let _permit = permit;
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
        }));
        while connections.next().now_or_never().flatten().is_some() {}
      }
      Either::Right(_) => {
        tracing::info!("Shutdown signal received, sending HTTP/3 GOAWAY...");
        break;
      }
    }
  }

  conn_shutdown.cancel();
  let deadline = std::time::Instant::now() + drain_timeout;
  if compio::time::timeout_at(deadline, async {
    while connections.next().await.is_some() {}
  })
  .await
  .is_err()
  {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining HTTP/3 connections",
      drain_timeout,
      connections.len()
    );
    for connection in connections {
      connection.cancel().await;
    }
  }

  endpoint.close(0u32.into(), b"server shutting down");
  let _ = compio::time::timeout_at(deadline, endpoint.shutdown()).await;
  tracing::info!("HTTP/3 server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&addr_str, "quic", true).await;
  Ok(())
}

/// Serves one HTTP/3 connection until the peer closes it or shutdown sends a
/// GOAWAY, then gives in-flight requests up to `goaway_grace` to finish.
async fn handle_connection(
  conn: Connection,
  router: Arc<Router>,
  remote_addr: SocketAddr,
  shutdown: CancellationToken,
  goaway_grace: Duration,
) -> Result<(), BoxError> {
  let mut h3_conn = h3::server::Connection::<_, Bytes>::new(conn).await?;
  let mut requests = FuturesUnordered::new();

  loop {
    let accepted = {
      let accept = std::pin::pin!(h3_conn.accept());
      let cancelled = std::pin::pin!(shutdown.cancelled());
      match futures_util::future::select(accept, cancelled).await {
        Either::Left((result, _)) => Some(result),
        Either::Right(_) => None,
      }
    };
    match accepted {
      Some(Ok(Some(resolver))) => {
        let router = router.clone();
        requests.push(compio::runtime::spawn(async move {
          match resolver.resolve_request().await {
            Ok((req, stream)) => {
              if let Err(e) = handle_request(req, stream, router, remote_addr).await {
                tracing::error!("HTTP/3 request error: {e}");
              }
            }
            Err(e) => {
              tracing::error!("HTTP/3 request resolve error: {e}");
            }
          }
        }));
        while requests.next().now_or_never().flatten().is_some() {}
      }
      Some(Ok(None)) => break,
      Some(Err(e)) => {
        tracing::error!("HTTP/3 accept error: {e}");
        break;
      }
      None => {
        // GOAWAY(0): the peer must not start new requests, while streams
        // already in flight keep draining below.
        if let Err(e) = h3_conn.shutdown(0).await {
          tracing::debug!("HTTP/3 GOAWAY error: {e}");
        }
        break;
      }
    }
  }

  if compio::time::timeout(goaway_grace, async {
    while requests.next().await.is_some() {}
  })
  .await
  .is_err()
  {
    tracing::debug!(
      "HTTP/3 connection grace ({:?}) elapsed; aborting {} request task(s)",
      goaway_grace,
      requests.len()
    );
    for request in requests {
      request.cancel().await;
    }
  }

  Ok(())
}
