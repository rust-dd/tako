//! HTTP-over-Unix-domain-socket server: the public `serve_unix_http*` entry
//! points and the shared accept/serve loop that dispatches into the router.

use std::convert::Infallible;
use std::future::Future;
use std::path::Path;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;
use tokio::task::JoinSet;

use super::listener::UnixPeerAddr;
use super::listener::bind_unix_listener;
use super::listener::is_abstract_path;
use crate::ServerConfig;

/// Starts an HTTP server over a Unix domain socket.
///
/// Ideal for production deployments behind a reverse proxy (nginx, `HAProxy`)
/// where the app communicates via a local socket file instead of TCP.
pub async fn serve_unix_http(path: impl AsRef<Path>, router: Router) {
  if let Err(e) = run_http(
    path.as_ref(),
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("Unix HTTP server error: {e}");
  }
}

/// Starts an HTTP server over a Unix domain socket with graceful shutdown.
pub async fn serve_unix_http_with_shutdown(
  path: impl AsRef<Path>,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
) {
  if let Err(e) = run_http(path.as_ref(), router, Some(signal), ServerConfig::default()).await {
    tracing::error!("Unix HTTP server error: {e}");
  }
}

/// Like [`serve_unix_http`] with caller-supplied [`ServerConfig`].
pub async fn serve_unix_http_with_config(
  path: impl AsRef<Path>,
  router: Router,
  config: ServerConfig,
) {
  if let Err(e) = run_http(
    path.as_ref(),
    router,
    None::<std::future::Pending<()>>,
    config,
  )
  .await
  {
    tracing::error!("Unix HTTP server error: {e}");
  }
}

/// Like [`serve_unix_http_with_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_unix_http_with_shutdown_and_config(
  path: impl AsRef<Path>,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run_http(path.as_ref(), router, Some(signal), config).await {
    tracing::error!("Unix HTTP server error: {e}");
  }
}

async fn run_http(
  path: &Path,
  router: Router,
  signal: Option<impl Future<Output = ()> + Send + 'static>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let listener = bind_unix_listener(path).await?;
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  tracing::debug!("Tako Unix HTTP listening on {}", path.display());

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
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
      result = listener.accept() => {
        let (stream, addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("Unix accept failed: {err}; backing off");
            accept_backoff.sleep_and_grow().await;
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
        let io = hyper_util::rt::TokioIo::new(stream);
        let router = router.clone();

        let peer_addr = UnixPeerAddr {
          path: addr.as_pathname().map(std::path::Path::to_path_buf),
        };

        join_set.spawn(async move {
          let svc = service_fn(move |mut req| {
            let router = router.clone();
            let peer_addr = peer_addr.clone();
            async move {
              let conn_info = ConnInfo::unix(peer_addr.path.clone());
              req.extensions_mut().insert(peer_addr);
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
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message on Unix socket: {err}");
            } else {
              tracing::error!("Error serving Unix HTTP connection: {err}");
            }
          }

          drop(permit);
        });
      }
      () = cancel.cancelled() => {
        tracing::info!("Unix HTTP server shutting down...");
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
    join_set.abort_all();
  }

  // Filesystem-backed paths get the socket file removed on shutdown so a
  // subsequent run can re-bind cleanly. Abstract sockets disappear with the
  // last reference, so there's nothing to clean.
  if !is_abstract_path(path) {
    let _ = std::fs::remove_file(path);
  }
  tracing::info!("Unix HTTP server shut down gracefully");
  Ok(())
}
