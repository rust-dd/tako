//! HTTP listener/acceptor that parses PROXY protocol headers per connection.

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::ConnectionTimer;
use tako_rs_core::server_support::connection_router;
use tako_rs_core::server_support::drive_connection;
use tako_rs_core::types::BoxError;
use tokio::task::JoinSet;

use super::apply_to_request;
use super::read_proxy_protocol;
use crate::ServerConfig;

/// Build an RFC 7239 `Forwarded` header value from the PROXY-protocol-supplied
/// peer address. IPv6 addresses get bracketed per the RFC's `node` ABNF.
/// Starts an HTTP server that parses PROXY protocol headers on each connection.
///
/// The real client address from the PROXY header is inserted into request
/// extensions as `SocketAddr` (overriding the TCP peer address). The raw
/// `ProxyHeader` is also available via `req.extensions().get::<ProxyHeader>()`.
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_http_with_proxy_protocol(listener: tokio::net::TcpListener, router: Router) {
  if let Err(e) = run_proxy_http(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Starts an HTTP server with PROXY protocol support and graceful shutdown.
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_http_with_proxy_protocol_and_shutdown(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Like [`serve_http_with_proxy_protocol`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_http_with_proxy_protocol_and_config(
  listener: tokio::net::TcpListener,
  router: Router,
  config: ServerConfig,
) {
  if let Err(e) = run_proxy_http(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Like [`serve_http_with_proxy_protocol_and_shutdown`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_http_with_proxy_protocol_shutdown_and_config(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal), config).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

pub(crate) async fn run_proxy_http(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  tracing::debug!(
    "Tako PROXY protocol HTTP listening on {}",
    listener.local_addr()?
  );

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
  let proxy_read_timeout = config.proxy_read_timeout;
  let cancel = tokio_util::sync::CancellationToken::new();
  let mut signal_tasks = JoinSet::new();
  if let Some(s) = signal {
    let cancel_for_signal = cancel.clone();
    signal_tasks.spawn(async move {
      s.await;
      cancel_for_signal.cancel();
    });
  }

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (mut stream, _tcp_addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("PROXY accept failed: {err}; backing off");
            tokio::select! {
              () = cancel.cancelled() => break,
              () = accept_backoff.sleep_and_grow() => {},
            }
            continue;
          }
        };
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
        let _ = stream.set_nodelay(true);
        let router = connection_router(&router);

        let conn_cancel = cancel.clone();
        join_set.spawn(async move {

          let handshake = tokio::select! {
            biased;
            () = conn_cancel.cancelled() => return,
            result = tokio::time::timeout(proxy_read_timeout, read_proxy_protocol(&mut stream)) => result,
          };
          let proxy_header = match handshake {
              Ok(Ok(h)) => h,
              Ok(Err(e)) => {
                tracing::warn!("Failed to parse PROXY protocol: {e}");
                return;
              }
              Err(_) => {
                tracing::warn!(
                  "PROXY protocol read deadline ({:?}) elapsed; dropping connection",
                  proxy_read_timeout,
                );
                return;
              }
            };

          let io = hyper_util::rt::TokioIo::new(stream);

          let svc = service_fn(move |mut req| {
            apply_to_request(&mut req, &proxy_header);
            let router = router.clone();
            async move {
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.timer(ConnectionTimer::new());
          http.header_read_timeout(header_read_timeout);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = drive_connection(conn, conn_cancel.cancelled(), hyper::server::conn::http1::UpgradeableConnection::graceful_shutdown).await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message on PROXY protocol connection: {err}");
            } else {
              tracing::error!("Error serving PROXY protocol connection: {err}");
            }
          }

          drop(permit);
        });
        while join_set.try_join_next().is_some() {}
      }
      () = cancel.cancelled() => {
        tracing::info!("PROXY protocol HTTP server shutting down...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout exceeded, aborting {} remaining connections",
      join_set.len()
    );
    join_set.shutdown().await;
  }

  tracing::info!("PROXY protocol HTTP server shut down gracefully");
  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_stopped(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;
  Ok(())
}
