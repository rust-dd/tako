//! Raw Unix domain socket serve loops: a plain accept-and-dispatch server and
//! its graceful-shutdown variants that drain in-flight connections.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use super::listener::bind_unix_listener;

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
  let listener = bind_unix_listener(path).await?;
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
/// In-flight connections are drained with a 30 second timeout. Use
/// [`serve_unix_with_shutdown_and_drain`] to override this bound.
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
  serve_unix_with_shutdown_and_drain(path, handler, signal, Duration::from_secs(30)).await
}

/// Same as [`serve_unix_with_shutdown`] but with an explicit drain timeout.
pub async fn serve_unix_with_shutdown_and_drain<F, S>(
  path: impl AsRef<Path>,
  handler: F,
  signal: S,
  drain_timeout: Duration,
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
  let listener = bind_unix_listener(path).await?;
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

  let _ = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  })
  .await;

  Ok(())
}
