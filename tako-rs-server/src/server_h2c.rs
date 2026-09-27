#![cfg(feature = "http2")]
#![cfg_attr(docsrs, doc(cfg(feature = "http2")))]

//! HTTP/2 cleartext (h2c) server, prior-knowledge mode.
//!
//! For deployments where a reverse proxy (Envoy, nginx, `HAProxy`) speaks HTTP/2
//! to the upstream over plain TCP — there is no TLS handshake or HTTP/1
//! Upgrade negotiation. Clients open a TCP connection and immediately send the
//! HTTP/2 connection preface; the server reads frames straight away.
//!
//! Use [`serve_h2c`] for a default-config server, or [`serve_h2c_with_config`]
//! to supply a [`crate::ServerConfig`] (drain timeout, `max_connections`, h2 caps).

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http2;
use hyper::service::service_fn;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::server_support::drive_connection;
#[cfg(feature = "signals")]
use tako_rs_core::signals::transport as signal_tx;
use tako_rs_core::types::BoxError;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::ServerConfig;

/// Starts an h2c server with default [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_h2c(listener: TcpListener, router: Router) {
  if let Err(e) = run(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("h2c server error: {e}");
  }
}

/// Starts an h2c server with graceful shutdown support.
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_h2c_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
) {
  if let Err(e) = run(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("h2c server error: {e}");
  }
}

/// Like [`serve_h2c`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_h2c_with_config(listener: TcpListener, router: Router, config: ServerConfig) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("h2c server error: {e}");
  }
}

/// Like [`serve_h2c_with_shutdown`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_h2c_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, Some(signal), config).await {
    tracing::error!("h2c server error: {e}");
  }
}

pub(crate) async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
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
          let svc = service_fn(move |mut req| {
            let router = router.clone();
            async move {
            req.extensions_mut().insert(addr);
            req.extensions_mut().insert(ConnInfo::h2c(addr));
            let resp = router.dispatch(req.map(TakoBody::incoming)).await;
            Ok::<_, Infallible>(resp)
          }});

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
