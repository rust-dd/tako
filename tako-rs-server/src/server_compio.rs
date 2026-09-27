use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use compio::net::TcpListener;
use cyper_core::HyperStream;
use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::future::Either;
use futures_util::stream::FuturesUnordered;
use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::drive_connection;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;

use crate::ServerConfig;

#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve(listener: TcpListener, router: Router) {
  if let Err(e) = run(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("Server error: {e}");
  }
}

/// Starts the Tako HTTP server (compio) with graceful shutdown support.
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_config(listener: TcpListener, router: Router, config: ServerConfig) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve_with_shutdown`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, Some(signal), config).await {
    tracing::error!("Server error: {e}");
  }
}

pub(crate) async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_rs_core::tracing::init_tracing();

  let router = Arc::new(router);
  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", false).await;

  tracing::debug!("Tako listening on {}", addr_str);

  let mut connections = FuturesUnordered::new();
  let drain_timeout = config.drain_timeout;
  let keep_alive = config.keep_alive;
  let header_read_timeout = config.header_read_timeout;

  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));

  let mut accept_backoff = config.accept_backoff;

  let cancel = tokio_util::sync::CancellationToken::new();
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
            tracing::warn!("compio accept failed: {err}; backing off");
            let d = accept_backoff.current_and_grow();

            let sleep = std::pin::pin!(compio::time::sleep(d));
            match futures_util::future::select(sleep, signal_fused.as_mut()).await {
              Either::Left(((), _)) => continue,
              Either::Right(_) => break,
            }
          }
        };

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

        let io = HyperStream::new_plain(stream);
        let router = router.clone();

        let conn_cancel = cancel.clone();
        connections.push(compio::runtime::spawn(async move {
          let _permit = permit;

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), false, None).await;

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(ConnInfo::tcp(addr));
              let response = router.dispatch(req.map(TakoBody::new)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http
            .timer(cyper_core::CompioTimer)
            .header_read_timeout(header_read_timeout);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = drive_connection(
            conn,
            conn_cancel.cancelled(),
            hyper::server::conn::http1::UpgradeableConnection::graceful_shutdown,
          )
          .await
          {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message: {err}");
            } else {
              tracing::error!("Error serving connection: {err}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), false, None).await;
        }));
        while connections.next().now_or_never().flatten().is_some() {}
      }
      Either::Right(_) => {
        tracing::info!("Shutdown signal received, draining connections...");
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

  tracing::info!("Server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&addr_str, "tcp", false).await;
  Ok(())
}
