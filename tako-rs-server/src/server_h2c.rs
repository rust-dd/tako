//! HTTP/2 prior-knowledge (h2c) accept loop, for proxies that speak HTTP/2 over plain TCP.

use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http2;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::drive_connection;
use tako_rs_core::server_support::http_service;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::ServerConfig;

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
  tracing::info!("Tako h2c (HTTP/2 cleartext) listening on {addr_str}");

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", false).await;

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config.max_connections.map(|n| Arc::new(Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let h2_max_concurrent_streams = config.h2_max_concurrent_streams;
  let h2_max_header_list_size = config.h2_max_header_list_size;
  let h2_max_send_buf_size = config.h2_max_send_buf_size;
  let h2_max_pending_accept_reset_streams = config.h2_max_pending_accept_reset_streams;
  let h2_keep_alive_interval = config.h2_keep_alive_interval;

  let cancel = tokio_util::sync::CancellationToken::new();
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
            tracing::warn!("h2c accept failed: {err}; backing off");
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
        let io = TokioIo::new(stream);

        let router = router.clone();
        let conn_cancel = cancel.clone();
        join_set.spawn(async move {
          let svc = http_service(router, ConnInfo::h2c(addr));

          let mut h2 = http2::Builder::new(TokioExecutor::new());
          h2.timer(hyper_util::rt::TokioTimer::new());
          h2.max_concurrent_streams(h2_max_concurrent_streams)
            .max_header_list_size(h2_max_header_list_size)
            .max_send_buf_size(h2_max_send_buf_size)
            .max_pending_accept_reset_streams(h2_max_pending_accept_reset_streams);
          if let Some(interval) = h2_keep_alive_interval {
            h2.keep_alive_interval(Some(interval));
          }

          if let Err(err) = drive_connection(h2.serve_connection(io, svc), conn_cancel.cancelled(), hyper::server::conn::http2::Connection::graceful_shutdown).await {
            tracing::warn!("h2c connection error: {err}");
          }

          drop(permit);
        });
        while join_set.try_join_next().is_some() {}
      }
      () = cancel.cancelled() => {
        tracing::info!("Shutdown signal received, draining h2c connections...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });
  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining h2c connections",
      drain_timeout,
      join_set.len()
    );
    join_set.shutdown().await;
  }

  tracing::info!("h2c server shut down gracefully");
  #[cfg(feature = "signals")]
  signal_tx::emit_server_stopped(&addr_str, "tcp", false).await;
  Ok(())
}
