//! Tokio HTTP/1 accept loop behind `Server::spawn_http`.

use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http1;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::ConnCtx;
use tako_rs_core::server_support::ConnDriver;
use tako_rs_core::server_support::Http1Io;
use tako_rs_core::server_support::Leased;
use tako_rs_core::server_support::ShutdownSignal;
use tako_rs_core::server_support::tcp_service;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::ServerConfig;

/// Serves until `signal` resolves, then drains connections for at most `config.drain_timeout`.
pub(crate) async fn run(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
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

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config.max_connections.map(|n| Arc::new(Semaphore::new(n)));
  let keep_alive = config.keep_alive;
  let header_read_timeout = config.header_read_timeout;
  let drain_timeout = config.drain_timeout;

  let cancel = CancellationToken::new();
  let stopping = Arc::new(ShutdownSignal::default());
  let mut signal_tasks = JoinSet::new();
  let cancel_for_signal = cancel.clone();
  signal_tasks.spawn(async move {
    signal.await;
    cancel_for_signal.cancel();
  });

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (stream, addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {

            tracing::warn!("accept failed: {err}; backing off");
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
        let ctx = Arc::new(ConnCtx::new(Arc::clone(&router), addr));
        let io = Http1Io::new(
          hyper_util::rt::TokioIo::new(Leased::new(stream, permit)),
          Arc::clone(&ctx.activity),
        );
        let stopping = Arc::clone(&stopping);
        join_set.spawn(async move {
          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), false, None).await;

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.pipeline_flush(true);
          let conn = http
            .serve_connection(io, tcp_service(Arc::clone(&ctx)))
            .with_upgrades();
          let driven = ConnDriver::new(
            conn,
            http1::UpgradeableConnection::graceful_shutdown,
            stopping,
            ctx,
            header_read_timeout,
          );
          match driven.await {
            Some(Err(err)) if err.is_incomplete_message() => {
              tracing::debug!("client disconnected mid-message: {err}");
            }
            Some(Err(err)) => tracing::error!("Error serving connection: {err}"),
            Some(Ok(())) => {}
            None => tracing::debug!("closed {addr} at the header deadline"),
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), false, None).await;
        });
        while join_set.try_join_next().is_some() {}
      }
      () = cancel.cancelled() => {
        tracing::info!("Shutdown signal received, draining connections...");
        break;
      }
    }
  }

  stopping.trigger();
  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining connections",
      drain_timeout,
      join_set.len()
    );
    join_set.shutdown().await;
  }

  tracing::info!("Server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&addr_str, "tcp", false).await;
  Ok(())
}
