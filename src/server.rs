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

#[cfg(feature = "signals")]
use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use crate::body::TakoBody;
use crate::router::Router;
#[cfg(feature = "signals")]
use crate::signals::Signal;
#[cfg(feature = "signals")]
use crate::signals::SignalArbiter;
#[cfg(feature = "signals")]
use crate::signals::ids;
use crate::types::BoxError;
#[cfg(feature = "signals")]
use crate::types::BuildHasher;

/// Default drain timeout for graceful shutdown (30 seconds).
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Starts the Tako HTTP server with the given listener and router.
pub async fn serve(listener: TcpListener, router: Router) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>).await {
    tracing::error!("Server error: {e}");
  }
}

/// Starts the Tako HTTP server with graceful shutdown support.
///
/// When the `signal` future completes, the server stops accepting new connections
/// and waits up to 30 seconds for in-flight requests to finish.
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, Some(signal)).await {
    tracing::error!("Server error: {e}");
  }
}

/// Runs the main server loop, accepting connections and dispatching requests.
async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
) -> Result<(), BoxError> {
  #[cfg(feature = "tako-tracing")]
  crate::tracing::init_tracing();

  let router = Arc::new(router);
  // Setup plugins
  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  let addr_str = listener.local_addr()?.to_string();

  #[cfg(feature = "signals")]
  {
    // Emit server.started
    let mut server_meta: HashMap<String, String, BuildHasher> =
      HashMap::with_hasher(BuildHasher::default());
    server_meta.insert("addr".to_string(), addr_str.clone());
    server_meta.insert("transport".to_string(), "tcp".to_string());
    server_meta.insert("tls".to_string(), "false".to_string());
    SignalArbiter::emit_app(Signal::with_metadata(ids::SERVER_STARTED, server_meta)).await;
  }

  tracing::debug!("Tako listening on {}", addr_str);

  let mut join_set = JoinSet::new();
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
        let (stream, addr) = result?;
        let io = hyper_util::rt::TokioIo::new(stream);
        let router = router.clone();

        join_set.spawn(async move {
          #[cfg(feature = "signals")]
          {
            let mut conn_open_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_open_meta.insert("remote_addr".to_string(), addr.to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_OPENED,
              conn_open_meta,
            ))
            .await;
          }

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            async move {
              #[cfg(feature = "signals")]
              let path = req.uri().path().to_string();
              #[cfg(feature = "signals")]
              let method = req.method().to_string();

              req.extensions_mut().insert(addr);

              #[cfg(feature = "signals")]
              {
                let mut req_meta: HashMap<String, String, BuildHasher> =
                  HashMap::with_hasher(BuildHasher::default());
                req_meta.insert("method".to_string(), method.clone());
                req_meta.insert("path".to_string(), path.clone());
                SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_STARTED, req_meta)).await;
              }

              let response = router.dispatch(req.map(TakoBody::new)).await;

              #[cfg(feature = "signals")]
              {
                let mut done_meta: HashMap<String, String, BuildHasher> =
                  HashMap::with_hasher(BuildHasher::default());
                done_meta.insert("method".to_string(), method);
                done_meta.insert("path".to_string(), path);
                done_meta.insert("status".to_string(), response.status().as_u16().to_string());
                SignalArbiter::emit_app(Signal::with_metadata(ids::REQUEST_COMPLETED, done_meta)).await;
              }

              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(true);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            tracing::error!("Error serving connection: {err}");
          }

          #[cfg(feature = "signals")]
          {
            let mut conn_close_meta: HashMap<String, String, BuildHasher> =
              HashMap::with_hasher(BuildHasher::default());
            conn_close_meta.insert("remote_addr".to_string(), addr.to_string());
            SignalArbiter::emit_app(Signal::with_metadata(
              ids::CONNECTION_CLOSED,
              conn_close_meta,
            ))
            .await;
          }
        });
      }
      () = &mut signal_fused => {
        tracing::info!("Shutdown signal received, draining connections...");
        break;
      }
    }
  }

  // Drain in-flight connections
  let drain = tokio::time::timeout(DEFAULT_DRAIN_TIMEOUT, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout ({:?}) exceeded, aborting {} remaining connections",
      DEFAULT_DRAIN_TIMEOUT,
      join_set.len()
    );
    join_set.abort_all();
  }

  tracing::info!("Server shut down gracefully");
  Ok(())
}
