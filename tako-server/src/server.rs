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
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::net::TcpListener;
use tokio::task::JoinSet;

use tako_core::body::TakoBody;
use tako_core::router::Router;
#[cfg(feature = "signals")]
use tako_core::signals::Signal;
#[cfg(feature = "signals")]
use tako_core::signals::SignalArbiter;
#[cfg(feature = "signals")]
use tako_core::signals::ids;
use tako_core::types::BoxError;

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
  {
    // Emit server.started
    SignalArbiter::emit_app(
      Signal::with_capacity(ids::SERVER_STARTED, 3)
        .meta("addr", addr_str.clone())
        .meta("transport", "tcp")
        .meta("tls", "false"),
    )
    .await;
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
        let _ = stream.set_nodelay(true);
        let io = hyper_util::rt::TokioIo::new(stream);

        join_set.spawn(async move {
          #[cfg(feature = "signals")]
          {
            SignalArbiter::emit_app(
              Signal::with_capacity(ids::CONNECTION_OPENED, 1)
                .meta("remote_addr", addr.to_string()),
            )
            .await;
          }

          // `router` is `&'static Router` — no Arc clone per connection or request.
          let svc = service_fn(move |mut req| async move {
              #[cfg(feature = "signals")]
              let path = req.uri().path().to_string();
              #[cfg(feature = "signals")]
              let method = req.method().to_string();

              req.extensions_mut().insert(addr);

              #[cfg(feature = "signals")]
              {
                SignalArbiter::emit_app(
                  Signal::with_capacity(ids::REQUEST_STARTED, 2)
                    .meta("method", method.clone())
                    .meta("path", path.clone()),
                )
                .await;
              }

              let response = router.dispatch(req.map(TakoBody::incoming)).await;

              #[cfg(feature = "signals")]
              {
                SignalArbiter::emit_app(
                  Signal::with_capacity(ids::REQUEST_COMPLETED, 3)
                    .meta("method", method)
                    .meta("path", path)
                    .meta("status", response.status().as_u16().to_string()),
                )
                .await;
              }

              Ok::<_, Infallible>(response)
          });

          let mut http = http1::Builder::new();
          http.keep_alive(true);
          http.pipeline_flush(true);
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
          {
            SignalArbiter::emit_app(
              Signal::with_capacity(ids::CONNECTION_CLOSED, 1)
                .meta("remote_addr", addr.to_string()),
            )
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
