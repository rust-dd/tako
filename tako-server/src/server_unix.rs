//! Unix Domain Socket server for local IPC and reverse proxy communication.
//!
//! Provides both raw Unix socket and HTTP-over-Unix-socket servers.
//! The HTTP variant is ideal for production deployments behind nginx/HAProxy
//! where the app communicates via a local socket file instead of TCP.
//!
//! # Examples
//!
//! ## Raw Unix socket (echo server)
//! ```rust,no_run
//! use tako::server_unix::serve_unix;
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!
//! # async fn example() -> std::io::Result<()> {
//! serve_unix("/tmp/tako.sock", |mut stream, _addr| {
//!     Box::pin(async move {
//!         let mut buf = vec![0u8; 4096];
//!         let n = stream.read(&mut buf).await?;
//!         stream.write_all(&buf[..n]).await?;
//!         Ok(())
//!     })
//! }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## HTTP over Unix socket
//! ```rust,no_run
//! use tako::server_unix::serve_unix_http;
//! use tako::router::Router;
//!
//! # async fn example() -> std::io::Result<()> {
//! let router = Router::new();
//! serve_unix_http("/tmp/tako-http.sock", router).await;
//! # Ok(())
//! # }
//! ```

use std::convert::Infallible;
use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::task::JoinSet;

use tako_core::body::TakoBody;
use tako_core::router::Router;
use tako_core::types::BoxError;

/// Default drain timeout for graceful shutdown (30 seconds).
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// Peer address information for Unix domain socket connections.
///
/// Inserted into request extensions for HTTP-over-UDS connections.
/// Handlers can access it via `req.extensions().get::<UnixPeerAddr>()`.
#[derive(Debug, Clone)]
pub struct UnixPeerAddr {
  /// The filesystem path of the peer socket, if available.
  /// Most client connections are unnamed (None).
  pub path: Option<std::path::PathBuf>,
}

/// Starts a raw Unix domain socket server.
///
/// Each accepted connection is dispatched to the handler with the stream
/// and the peer's socket address.
pub async fn serve_unix<F>(path: impl AsRef<Path>, handler: F) -> std::io::Result<()>
where
  F: Fn(
      tokio::net::UnixStream,
      tokio::net::unix::SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
    + Send
    + Sync
    + 'static,
{
  let path = path.as_ref();
  cleanup_stale_socket(path)?;

  let listener = tokio::net::UnixListener::bind(path)?;
  tracing::info!("Unix socket server listening on {}", path.display());

  let handler = Arc::new(handler);

  loop {
    let (stream, addr) = listener.accept().await?;
    let handler = Arc::clone(&handler);

    tokio::spawn(async move {
      if let Err(e) = handler(stream, addr).await {
        tracing::error!("Unix socket connection error: {e}");
      }
    });
  }
}

/// Starts a raw Unix domain socket server with a shutdown signal.
///
/// The server stops accepting new connections when the shutdown signal completes.
/// In-flight connections are drained with a 30 second timeout.
pub async fn serve_unix_with_shutdown<F, S>(
  path: impl AsRef<Path>,
  handler: F,
  signal: S,
) -> std::io::Result<()>
where
  F: Fn(
      tokio::net::UnixStream,
      tokio::net::unix::SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  let path = path.as_ref();
  cleanup_stale_socket(path)?;

  let listener = tokio::net::UnixListener::bind(path)?;
  tracing::info!("Unix socket server listening on {}", path.display());

  let handler = Arc::new(handler);
  let mut join_set = JoinSet::new();

  tokio::pin!(signal);

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (stream, addr) = result?;
        let handler = Arc::clone(&handler);

        join_set.spawn(async move {
          if let Err(e) = handler(stream, addr).await {
            tracing::error!("Unix socket connection error: {e}");
          }
        });
      }
      () = &mut signal => {
        tracing::info!("Unix socket server shutting down, draining {} connections", join_set.len());
        break;
      }
    }
  }

  let drain_timeout = Duration::from_secs(30);
  let _ = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  })
  .await;

  Ok(())
}

/// Starts an HTTP server over a Unix domain socket.
///
/// Ideal for production deployments behind a reverse proxy (nginx, HAProxy)
/// where the app communicates via a local socket file instead of TCP.
pub async fn serve_unix_http(path: impl AsRef<Path>, router: Router) {
  if let Err(e) = run_http(path.as_ref(), router, None::<std::future::Pending<()>>).await {
    tracing::error!("Unix HTTP server error: {e}");
  }
}

/// Starts an HTTP server over a Unix domain socket with graceful shutdown.
pub async fn serve_unix_http_with_shutdown(
  path: impl AsRef<Path>,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run_http(path.as_ref(), router, Some(signal)).await {
    tracing::error!("Unix HTTP server error: {e}");
  }
}

async fn run_http(
  path: &Path,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
) -> Result<(), BoxError> {
  cleanup_stale_socket(path)?;

  let listener = tokio::net::UnixListener::bind(path)?;
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  tracing::debug!("Tako Unix HTTP listening on {}", path.display());

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

        let peer_addr = UnixPeerAddr {
          path: addr.as_pathname().map(|p| p.to_path_buf()),
        };

        join_set.spawn(async move {
          let svc = service_fn(move |mut req| {
            let router = router.clone();
            let peer_addr = peer_addr.clone();
            async move {
              req.extensions_mut().insert(peer_addr);
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(true);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message on Unix socket: {err}");
            } else {
              tracing::error!("Error serving Unix HTTP connection: {err}");
            }
          }
        });
      }
      () = &mut signal_fused => {
        tracing::info!("Unix HTTP server shutting down...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(DEFAULT_DRAIN_TIMEOUT, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout exceeded, aborting {} remaining connections",
      join_set.len()
    );
    join_set.abort_all();
  }

  // Clean up socket file
  let _ = std::fs::remove_file(path);
  tracing::info!("Unix HTTP server shut down gracefully");
  Ok(())
}

/// Removes a stale socket file if it exists and is not actively in use.
fn cleanup_stale_socket(path: &Path) -> std::io::Result<()> {
  if path.exists() {
    // Try connecting to see if the socket is active
    match std::os::unix::net::UnixStream::connect(path) {
      Ok(_) => {
        // Socket is active — don't remove it
        return Err(std::io::Error::new(
          std::io::ErrorKind::AddrInUse,
          format!("Unix socket {} is already in use", path.display()),
        ));
      }
      Err(_) => {
        // Socket is stale — safe to remove
        std::fs::remove_file(path)?;
      }
    }
  }
  Ok(())
}
