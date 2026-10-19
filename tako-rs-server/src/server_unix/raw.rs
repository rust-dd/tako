//! Raw Unix domain socket servers on Tokio.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use super::listener::bind_unix_listener;

/// Runs `handler` for every accepted connection until an accept error, which it returns.
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

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(&path.to_string_lossy(), "unix", false)
    .await;

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

/// Stops accepting once `signal` resolves and drains connections for up to 30 seconds; use
/// [`serve_unix_with_shutdown_and_drain`] for another bound.
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

/// Like [`serve_unix_with_shutdown`] with an explicit drain timeout.
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

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(&path.to_string_lossy(), "unix", false)
    .await;

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
        while join_set.try_join_next().is_some() {}
      }
      () = &mut signal => {
        tracing::info!("Unix socket server shutting down, draining {} connections", join_set.len());
        break;
      }
    }
  }

  if tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  })
  .await
  .is_err()
  {
    join_set.shutdown().await;
  }

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_stopped(&path.to_string_lossy(), "unix", false)
    .await;
  Ok(())
}
