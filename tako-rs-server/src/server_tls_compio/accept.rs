//! The compio TLS accept/serve loop: per-connection handshake, protocol
//! dispatch (HTTP/1.1 and HTTP/2), graceful shutdown, and connection draining.

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use compio::net::TcpListener;
use compio::tls::TlsAcceptor;
use cyper_core::HyperStream;
use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::future::Either;
use futures_util::stream::FuturesUnordered;
use hyper::server::conn::http1;
#[cfg(feature = "http2")]
use hyper::server::conn::http2;
use hyper::service::service_fn;
use rustls::ServerConfig as RustlsServerConfig;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::TlsInfo;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::drive_connection;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio_util::sync::CancellationToken;

use crate::ServerConfig;
#[cfg(feature = "http2")]
use crate::server_tls_compio::executor::CompioH2Executor;
#[cfg(feature = "http2")]
use crate::server_tls_compio::executor::CompioH2Timer;
#[cfg(feature = "http2")]
use crate::server_tls_compio::executor::ServiceSendWrapper;

/// Variant of [`run`](super::run) that accepts a pre-built `Arc<rustls::ServerConfig>`.
pub async fn run_with_config(
  listener: TcpListener,
  router: Router,
  tls_config: Arc<RustlsServerConfig>,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let acceptor = TlsAcceptor::from(tls_config);
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", true).await;

  tracing::info!("Tako TLS listening on {}", addr_str);

  let mut connections = FuturesUnordered::new();
  let drain_timeout = config.drain_timeout;
  let tls_handshake_timeout = config.tls_handshake_timeout;
  let keep_alive = config.keep_alive;
  let header_read_timeout = config.header_read_timeout;
  #[cfg(feature = "http2")]
  let h2_max_concurrent_streams = config.h2_max_concurrent_streams;
  #[cfg(feature = "http2")]
  let h2_max_header_list_size = config.h2_max_header_list_size;
  #[cfg(feature = "http2")]
  let h2_max_send_buf_size = config.h2_max_send_buf_size;
  #[cfg(feature = "http2")]
  let h2_max_pending_accept_reset_streams = config.h2_max_pending_accept_reset_streams;
  #[cfg(feature = "http2")]
  let h2_keep_alive_interval = config.h2_keep_alive_interval;

  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

  let mut accept_backoff = config.accept_backoff;

  let cancel = CancellationToken::new();
  let signal = signal.map(|s| Box::pin(s));
  let mut signal_fused = std::pin::pin!(async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  });

  loop {
    let accept = std::pin::pin!(listener.accept());
    match futures_util::future::select(accept, signal_fused.as_mut()).await {
      Either::Left((result, _)) => {
        let (stream, addr) = match result {
          Ok(v) => {
            accept_backoff.reset();
            v
          }
          Err(err) => {
            tracing::warn!("compio TLS accept failed: {err}; backing off");
            let d = accept_backoff.current_and_grow();

            let sleep = std::pin::pin!(compio::time::sleep(d));
            match futures_util::future::select(sleep, signal_fused.as_mut()).await {
              Either::Left(((), _)) => continue,
              Either::Right(_) => {
                cancel.cancel();
                break;
              }
            }
          }
        };

        let permit = if let Some(sem) = max_conn_semaphore.as_ref() {
          let acquire = std::pin::pin!(sem.clone().acquire_owned());
          match futures_util::future::select(acquire, signal_fused.as_mut()).await {
            Either::Left((Ok(p), _)) => Some(p),
            Either::Left((Err(_), _)) => continue,
            Either::Right(_) => {
              cancel.cancel();
              break;
            }
          }
        } else {
          None
        };

        let acceptor = acceptor.clone();
        let router = router.clone();

        let conn_cancel = cancel.clone();

        connections.push(compio::runtime::spawn(async move {
          let _permit = permit;

          let handshake_deadline = std::pin::pin!(compio::time::sleep(tls_handshake_timeout));
          let shutdown_wait = std::pin::pin!(conn_cancel.cancelled());
          let deadline_or_shutdown = std::pin::pin!(futures_util::future::select(
            handshake_deadline,
            shutdown_wait
          ));
          let accept_fut = std::pin::pin!(acceptor.accept(stream));
          let tls_stream =
            match futures_util::future::select(accept_fut, deadline_or_shutdown).await {
              Either::Left((Ok(s), _)) => s,
              Either::Left((Err(e), _)) => {
                tracing::debug!("TLS error: {e}");
                return;
              }
              Either::Right((Either::Left(_), _)) => {
                tracing::warn!("TLS handshake timeout after {tls_handshake_timeout:?} from {addr}");
                return;
              }
              Either::Right((Either::Right(_), _)) => {
                tracing::debug!("TLS handshake aborted by shutdown from {addr}");
                return;
              }
            };

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), true, None).await;

          let alpn_proto = tls_stream
            .negotiated_alpn()
            .map(|alpn| bytes::Bytes::copy_from_slice(&alpn));
          let is_h2 = matches!(alpn_proto.as_deref(), Some(b"h2"));

          let conn_info = if is_h2 {
            ConnInfo::h2_tls(
              addr,
              TlsInfo {
                alpn: alpn_proto.clone(),
                sni: None,
                version: None,
              },
            )
          } else {
            ConnInfo::h1_tls(
              addr,
              TlsInfo {
                alpn: alpn_proto.clone(),
                sni: None,
                version: None,
              },
            )
          };

          #[cfg(feature = "http2")]
          let proto = alpn_proto;

          let io = HyperStream::new_tls(tls_stream);

          let svc = service_fn(move |mut req| {
            let r = router.clone();
            let conn_info = conn_info.clone();
            async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(conn_info);
              let response = r.dispatch(req.map(TakoBody::new)).await;
              Ok::<_, Infallible>(response)
            }
          });

          #[cfg(feature = "http2")]
          if proto.as_deref() == Some(b"h2") {
            let mut h2 = http2::Builder::new(CompioH2Executor);
            h2.timer(CompioH2Timer)
              .max_concurrent_streams(h2_max_concurrent_streams)
              .max_header_list_size(h2_max_header_list_size)
              .max_send_buf_size(h2_max_send_buf_size)
              .max_pending_accept_reset_streams(h2_max_pending_accept_reset_streams);
            if let Some(interval) = h2_keep_alive_interval {
              h2.keep_alive_interval(Some(interval));
            }

            if let Err(e) = drive_connection(
              h2.serve_connection(io, ServiceSendWrapper::new(svc)),
              conn_cancel.cancelled(),
              hyper::server::conn::http2::Connection::graceful_shutdown,
            )
            .await
            {
              tracing::debug!("HTTP/2 error: {e}");
            }

            #[cfg(feature = "signals")]
            signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;

            return;
          }

          let mut h1 = http1::Builder::new();
          h1.keep_alive(keep_alive);
          h1.timer(cyper_core::CompioTimer)
            .header_read_timeout(header_read_timeout);

          if let Err(e) = drive_connection(
            h1.serve_connection(io, svc).with_upgrades(),
            conn_cancel.cancelled(),
            hyper::server::conn::http1::UpgradeableConnection::graceful_shutdown,
          )
          .await
          {
            if e.is_incomplete_message() {
              tracing::debug!("TLS HTTP/1.1 client disconnected mid-message: {e}");
            } else {
              tracing::error!("HTTP/1.1 error: {e}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), true, None).await;
        }));
        while connections.next().now_or_never().flatten().is_some() {}
      }
      Either::Right(_) => {
        cancel.cancel();
        tracing::info!("Shutdown signal received, draining TLS connections...");
        break;
      }
    }
  }

  cancel.cancel();
  if compio::time::timeout(drain_timeout, async {
    while connections.next().await.is_some() {}
  })
  .await
  .is_err()
  {
    tracing::warn!("drain timeout exceeded; cancelling remaining connections");
    for connection in connections {
      connection.cancel().await;
    }
  }

  tracing::info!("TLS server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&addr_str, "tcp", true).await;
  Ok(())
}
