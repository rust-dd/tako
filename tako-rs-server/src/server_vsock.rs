//! Linux vsock HTTP/1 accept loop, for guest VM and host traffic that bypasses the network stack.

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::conn_info::Transport;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::ConnectionTimer;
use tako_rs_core::server_support::Leased;
use tako_rs_core::server_support::connection_router;
use tako_rs_core::server_support::drive_connection;
use tako_rs_core::types::BoxError;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_vsock::VsockListener;

use crate::ServerConfig;

pub(crate) async fn run_listener(
  listener: VsockListener,
  cid: u32,
  port: u32,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once()?;

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(&format!("{cid}:{port}"), "vsock", false)
    .await;

  tracing::info!("Tako vsock HTTP listening on cid={cid} port={port}");

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config.max_connections.map(|n| Arc::new(Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
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
        let (stream, peer) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("vsock accept failed: {err}; backing off");
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
        let io = hyper_util::rt::TokioIo::new(Leased::new(stream, permit));
        let router = connection_router(&router);

        let conn_cancel = cancel.clone();
        join_set.spawn(async move {
          let peer_label = format!("vsock:{}:{}", peer.cid(), peer.port());
          let svc = service_fn(move |mut req| {
            let router = router.clone();
            let peer_label = peer_label.clone();
            async move {
              let conn_info = ConnInfo {
                peer: PeerAddr::Other(peer_label.clone()),
                transport: Transport::Http1,
                tls: None,
              };
              req.extensions_mut().insert(conn_info);
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.timer(ConnectionTimer::new());
          http.header_read_timeout(header_read_timeout);

          if let Err(err) = drive_connection(http.serve_connection(io, svc).with_upgrades(), conn_cancel.cancelled(), hyper::server::conn::http1::UpgradeableConnection::graceful_shutdown).await {
            if err.is_incomplete_message() {
              tracing::debug!("vsock client disconnected mid-message: {err}");
            } else {
              tracing::error!("vsock HTTP error: {err}");
            }
          }
        });
        while join_set.try_join_next().is_some() {}
      }
      () = cancel.cancelled() => {
        tracing::info!("vsock HTTP server shutting down...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });
  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining vsock connections",
      drain_timeout,
      join_set.len()
    );
    join_set.shutdown().await;
  }

  tracing::info!("vsock HTTP server shut down gracefully");
  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_stopped(&format!("{cid}:{port}"), "vsock", false)
    .await;
  Ok(())
}
