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

use hyper::{Request, server::conn::http1, service::service_fn};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::router::Router;
use crate::types::BoxError;

/// Starts the Tako HTTP server with the given listener and router.
///
/// This function initializes tracing (if enabled), sets up plugins (if enabled),
/// and enters the main server loop to accept and handle incoming connections.
/// Each connection is handled in a separate tokio task for concurrent processing.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::{serve, router::Router};
/// use tokio::net::TcpListener;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let listener = TcpListener::bind("127.0.0.1:8080").await?;
/// let router = Router::new();
/// serve(listener, router).await;
/// # Ok(())
/// # }
/// ```
pub async fn serve(listener: TcpListener, router: Router) {
    run(listener, router).await.unwrap();
}

/// Runs the main server loop, accepting connections and dispatching requests.
///
/// This function handles the core server logic including connection acceptance,
/// task spawning for concurrent request handling, and request dispatching through
/// the router. It also handles HTTP/1.1 protocol specifics and connection upgrades.
///
/// # Errors
///
/// Returns an error if the server fails to bind to the listener address, accept
/// incoming connections, or encounters other I/O related issues.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::{router::Router};
/// use tokio::net::TcpListener;
/// use tako::server::run;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let listener = TcpListener::bind("127.0.0.1:8080").await?;
/// let router = Router::new();
/// run(listener, router).await?;
/// # Ok(())
/// # }
/// ```
async fn run(listener: TcpListener, router: Router) -> Result<(), BoxError> {
    #[cfg(feature = "tako-tracing")]
    crate::tracing::init_tracing();

    let router = Arc::new(router);
    // Setup plugins
    #[cfg(feature = "plugins")]
    router.setup_plugins_once();

    println!("Tako listening on {}", listener.local_addr()?);

    loop {
        let (stream, addr) = listener.accept().await?;
        let io = hyper_util::rt::TokioIo::new(stream);
        let router = router.clone();

        // Spawn a new task to handle each incoming connection.
        tokio::spawn(async move {
            let svc = Arc::new(service_fn(move |mut req: Request<_>| {
                let router = router.clone();
                async move {
                    req.extensions_mut().insert(addr);
                    Ok::<_, Infallible>(router.dispatch(req).await)
                }
            }));

            let mut http = http1::Builder::new();
            http.keep_alive(true);
            // Serve the connection using HTTP/1.1 with support for upgrades.
            let conn = http.serve_connection(io, svc).with_upgrades();

            if let Err(err) = conn.await {
                eprintln!("Error serving connection: {err}");
            }
        });
    }
}
