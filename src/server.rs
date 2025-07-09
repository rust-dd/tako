/// This module provides the `serve` function and related utilities for running a Tako HTTP server.
///
/// The server is built on top of Hyper and supports asynchronous request handling.
use hyper::{Request, server::conn::http1, service::service_fn};
use std::convert::Infallible;
use std::sync::Arc;
use tokio::net::TcpListener;

use crate::router::Router;
use crate::types::BoxError;

/// Starts the Tako HTTP server.
///
/// # Arguments
///
/// * `listener` - A `TcpListener` that listens for incoming connections.
/// * `router` - A `Router` instance that defines the routes and handlers for the server.
///
/// # Example
///
/// ```no_run
/// use tako::router::Router;
/// use tokio::net::TcpListener;
/// use tako::server::serve;
///
/// #[tokio::main]
/// async fn main() {
///     let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();
///     let router = Router::new();
///     serve(listener, router).await;
/// }
/// ```
pub async fn serve(listener: TcpListener, router: Router) {
    run(listener, router).await.unwrap();
}

/// Internal function to run the server loop.
///
/// This function accepts incoming connections, spawns tasks to handle them,
/// and uses the provided router to dispatch requests.
///
/// # Arguments
///
/// * `listener` - A `TcpListener` for accepting connections.
/// * `router` - A `Router` instance for handling requests.
///
/// # Returns
///
/// A `Result` indicating success or failure.
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
