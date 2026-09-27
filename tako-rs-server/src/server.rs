//! HTTP server implementation and lifecycle management.
//!
//! This module provides the core server functionality for Tako, built on top of Hyper.
//! It handles incoming TCP connections, dispatches requests through the router, and
//! manages the server lifecycle. The main entry point is the `serve` function which
//! starts an HTTP server with the provided listener and router configuration.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::{serve, router::Router, Method, responder::Responder, types::Request};
//! use tokio::net::TcpListener;
//!
//! async fn hello(_: Request) -> impl Responder {
//!     "Hello, World!".into_response()
//! }
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let listener = TcpListener::bind("127.0.0.1:8080").await?;
//! let mut router = Router::new();
//! router.route(Method::GET, "/", hello);
//! serve(listener, router).await;
//! # Ok(())
//! # }
//! ```

use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
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
use tokio_util::sync::CancellationToken;

use crate::ServerConfig;

/// Starts the Tako HTTP server with the given listener and router.
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

/// Starts the Tako HTTP server with graceful shutdown support.
///
/// When the `signal` future completes, the server stops accepting new connections
/// and waits up to `ServerConfig::drain_timeout` (default 30 s) for in-flight
/// requests to finish.
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
) {
  if let Err(e) = run(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve`] but with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_config(listener: TcpListener, router: Router, config: ServerConfig) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve_with_shutdown`] but with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()> + Send + 'static,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, Some(signal), config).await {
    tracing::error!("Server error: {e}");
  }
}

/// Runs the main server loop, accepting connections and dispatching requests.
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
        let io = hyper_util::rt::TokioIo::new(stream);

        let router = router.clone();
        let conn_cancel = cancel.clone();
        join_set.spawn(async move {
          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), false, None).await;

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(ConnInfo::tcp(addr));
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
          }});

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.pipeline_flush(true);

          http.timer(hyper_util::rt::TokioTimer::new());
          http.header_read_timeout(header_read_timeout);

          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = drive_connection(conn, conn_cancel.cancelled(), hyper::server::conn::http1::UpgradeableConnection::graceful_shutdown).await {

            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message: {err}");
            } else {
              tracing::error!("Error serving connection: {err}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), false, None).await;

          drop(permit);
        });
        while join_set.try_join_next().is_some() {}
      }
      () = cancel.cancelled() => {
        tracing::info!("Shutdown signal received, draining connections...");
        break;
      }
    }
  }

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
