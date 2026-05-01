#![cfg(all(target_os = "linux", feature = "vsock"))]
#![cfg_attr(docsrs, doc(cfg(all(target_os = "linux", feature = "vsock"))))]

//! Linux vsock (VM ⇄ host) HTTP server.
//!
//! `vsock` provides a socket family for communication between a guest VM and
//! its host without going through the network stack. The address pair is
//! `(CID, port)` where the well-known CIDs are exposed as
//! [`tokio_vsock::VMADDR_CID_HOST`] (host side) and
//! [`tokio_vsock::VMADDR_CID_ANY`] (bind to any guest interface).
//!
//! Common deployments:
//! - **Confidential VM enclave** exposing an internal management API to the
//!   host without exposing it to the network.
//! - **Host-side agent** receiving telemetry from per-VM agents over a single
//!   well-known port.
//!
//! # Example
//!
//! ```rust,no_run
//! # #[cfg(all(target_os = "linux", feature = "vsock"))]
//! # async fn _ex() {
//! use tako_server::server_vsock::serve_vsock_http;
//! use tako_core::router::Router;
//! use tokio_vsock::VMADDR_CID_ANY;
//!
//! let router = Router::new();
//! serve_vsock_http(VMADDR_CID_ANY, 5005, router).await;
//! # }
//! ```

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;
use tokio_vsock::{VsockAddr, VsockListener};

use tako_core::body::TakoBody;
use tako_core::conn_info::{ConnInfo, PeerAddr, Transport};
use tako_core::router::Router;
use tako_core::types::BoxError;

use crate::ServerConfig;

/// Starts an HTTP server bound to a vsock `(cid, port)` pair.
pub async fn serve_vsock_http(cid: u32, port: u32, router: Router) {
  if let Err(e) = run(
    cid,
    port,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("vsock HTTP server error: {e}");
  }
}

/// Like [`serve_vsock_http`] with graceful shutdown.
pub async fn serve_vsock_http_with_shutdown(
  cid: u32,
  port: u32,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(cid, port, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("vsock HTTP server error: {e}");
  }
}

/// Like [`serve_vsock_http`] with caller-supplied [`ServerConfig`].
pub async fn serve_vsock_http_with_config(
  cid: u32,
  port: u32,
  router: Router,
  config: ServerConfig,
) {
  if let Err(e) = run(cid, port, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("vsock HTTP server error: {e}");
  }
}

/// Like [`serve_vsock_http_with_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_vsock_http_with_shutdown_and_config(
  cid: u32,
  port: u32,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(cid, port, router, Some(signal), config).await {
    tracing::error!("vsock HTTP server error: {e}");
  }
}

async fn run(
  cid: u32,
  port: u32,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  let listener = VsockListener::bind(VsockAddr::new(cid, port))?;
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  tracing::info!("Tako vsock HTTP listening on cid={cid} port={port}");

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
  let signal = signal.map(|s| Box::pin(s));
  let signal_fused = async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  };
  tokio::pin!(signal_fused);

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (stream, peer) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("vsock accept failed: {err}; backing off");
            accept_backoff.sleep_and_grow().await;
            continue;
          }
        };
        let permit = if let Some(sem) = &max_conn_semaphore {
          match sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => continue,
          }
        } else {
          None
        };
        let io = hyper_util::rt::TokioIo::new(stream);
        let router = router.clone();

        join_set.spawn(async move {
          let peer_label = format!("vsock:{}:{}", peer.cid(), peer.port());
          let svc = service_fn(move |mut req| {
            let router = router.clone();
            let peer_label = peer_label.clone();
            async move {
              let conn_info = ConnInfo {
                peer: PeerAddr::Other(peer_label.clone()),
                local: None,
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
          http.timer(hyper_util::rt::TokioTimer::new());
          if let Some(t) = header_read_timeout {
            http.header_read_timeout(t);
          }

          if let Err(err) = http.serve_connection(io, svc).with_upgrades().await {
            if err.is_incomplete_message() {
              tracing::debug!("vsock client disconnected mid-message: {err}");
            } else {
              tracing::error!("vsock HTTP error: {err}");
            }
          }

          drop(permit);
        });
      }
      () = &mut signal_fused => {
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
    join_set.abort_all();
  }

  tracing::info!("vsock HTTP server shut down gracefully");
  Ok(())
}
