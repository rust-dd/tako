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
use tako_core::body::TakoBody;
use tako_core::conn_info::ConnInfo;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::transport as signal_tx;
use tako_core::types::BoxError;
use tokio::net::TcpListener;
use tokio::sync::Semaphore;
use tokio::task::JoinSet;

use crate::ServerConfig;

/// Starts the Tako HTTP server with the given listener and router.
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
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve`] but with caller-supplied [`ServerConfig`].
pub async fn serve_with_config(listener: TcpListener, router: Router, config: ServerConfig) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve_with_shutdown`] but with caller-supplied [`ServerConfig`].
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

/// Runs the main server loop, accepting connections and dispatching requests.
async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  tako_core::tracing::init_tracing();

  // Leak the router into a `&'static` reference to eliminate all Arc
  // refcount bumps on the per-connection and per-request hot paths.
  // The allocation is reclaimed when the process exits.
  let router: &'static Router = Box::leak(Box::new(router));

  // Setup plugins
  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  signal_tx::emit_server_started(&addr_str, "tcp", false).await;

  tracing::debug!("Tako listening on {}", addr_str);

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config.max_connections.map(|n| Arc::new(Semaphore::new(n)));
  let keep_alive = config.keep_alive;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive_timeout = config.keep_alive_timeout;
  let drain_timeout = config.drain_timeout;
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
        let (stream, addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            // Accept errors (typically EMFILE/ENFILE under FD pressure, or
            // ConnectionAborted under load) are not fatal — log, back off, retry.
            tracing::warn!("accept failed: {err}; backing off");
            accept_backoff.sleep_and_grow().await;
            continue;
          }
        };

        // Optional connection cap: park here until a permit is available so
        // we exert backpressure on the kernel listen queue rather than
        // accepting unbounded work.
        let permit = if let Some(sem) = &max_conn_semaphore {
          match sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => continue,
          }
        } else {
          None
        };

        let _ = stream.set_nodelay(true);
        let io = hyper_util::rt::TokioIo::new(stream);

        join_set.spawn(async move {
          #[cfg(feature = "signals")]
          signal_tx::emit_connection_opened(&addr.to_string(), false, None).await;

          // `router` is `&'static Router` — no Arc clone per connection or request.
          // Per-request REQUEST_STARTED / REQUEST_COMPLETED signals fire from
          // inside Router::dispatch, so transports stay free of that boilerplate.
          let svc = service_fn(move |mut req| async move {
              req.extensions_mut().insert(addr);
              req.extensions_mut().insert(ConnInfo::tcp(addr));
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.pipeline_flush(true);
          // hyper requires a Timer when header_read_timeout is set; default
          // installs the tokio timer integration.
          http.timer(hyper_util::rt::TokioTimer::new());
          if let Some(t) = header_read_timeout {
            http.header_read_timeout(t);
          }
          if let Some(t) = keep_alive_timeout {
            // Hyper does not expose a keep-alive idle timeout knob on http1
            // builder yet; reserved for future plumb-through.
            let _ = t;
          }
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            // Hyper raises `IncompleteMessage` when the peer closes mid-request
            // or mid-response. This is normal traffic (keep-alive races, client
            // cancellation, NAT/proxy timeouts) and shouldn't pollute ERROR logs.
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message: {err}");
            } else {
              tracing::error!("Error serving connection: {err}");
            }
          }

          #[cfg(feature = "signals")]
          signal_tx::emit_connection_closed(&addr.to_string(), false, None).await;

          // Permit lives until here; dropping it returns a slot to the
          // max_connections semaphore so the next accept can proceed.
          drop(permit);
        });
      }
      () = &mut signal_fused => {
        tracing::info!("Shutdown signal received, draining connections...");
        break;
      }
    }
  }

  // Drain in-flight connections
  let drain = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining connections",
      drain_timeout,
      join_set.len()
    );
    join_set.abort_all();
  }

  tracing::info!("Server shut down gracefully");
  Ok(())
}
