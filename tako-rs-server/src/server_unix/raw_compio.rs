//! Raw Unix domain socket serve loops on the compio runtime.

use std::future::Future;
use std::path::Path;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::StreamExt;
use futures_util::future::Either;
use futures_util::stream::FuturesUnordered;
use tako_rs_core::conn_info::UnixPeerAddr;

use crate::server_compio::unix::bind_listener;
use crate::server_compio::unix::peer_addr;

/// Starts a raw Unix domain socket server (compio runtime).
///
/// Each accepted connection is dispatched to the handler with the stream
/// and the peer's address.
pub async fn serve_unix<F>(path: impl AsRef<Path>, handler: F) -> std::io::Result<()>
where
  F: Fn(compio::net::UnixStream, UnixPeerAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
{
  let path = path.as_ref();
  let listener = bind_listener(path).await?;
  tracing::info!("Unix socket server listening on {}", path.display());

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(&path.to_string_lossy(), "unix", false)
    .await;

  let handler = Arc::new(handler);

  loop {
    let (stream, addr) = listener.accept().await?;
    let peer = peer_addr(addr.as_pathname());
    let handler = Arc::clone(&handler);

    compio::runtime::spawn(async move {
      if let Err(e) = handler(stream, peer).await {
        tracing::error!("Unix socket connection error: {e}");
      }
    })
    .detach();
  }
}

/// Starts a raw Unix domain socket server with a shutdown signal (compio
/// runtime).
///
/// The server stops accepting new connections when the shutdown signal
/// completes. In-flight connections are drained with a 30 second timeout. Use
/// [`serve_unix_with_shutdown_and_drain`] to override this bound.
pub async fn serve_unix_with_shutdown<F, S>(
  path: impl AsRef<Path>,
  handler: F,
  signal: S,
) -> std::io::Result<()>
where
  F: Fn(compio::net::UnixStream, UnixPeerAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
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
  F: Fn(compio::net::UnixStream, UnixPeerAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  let path = path.as_ref();
  let listener = bind_listener(path).await?;
  tracing::info!("Unix socket server listening on {}", path.display());

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(&path.to_string_lossy(), "unix", false)
    .await;

  let handler = Arc::new(handler);
  let mut connections = FuturesUnordered::new();
  let mut signal = std::pin::pin!(signal);

  loop {
    let accept = std::pin::pin!(listener.accept());
    match futures_util::future::select(accept, &mut signal).await {
      Either::Left((result, _)) => {
        let (stream, addr) = result?;
        let peer = peer_addr(addr.as_pathname());
        let handler = Arc::clone(&handler);
        connections.push(compio::runtime::spawn(async move {
          if let Err(e) = handler(stream, peer).await {
            tracing::error!("Unix socket connection error: {e}");
          }
        }));
        while connections.next().now_or_never().flatten().is_some() {}
      }
      Either::Right(_) => {
        tracing::info!(
          "Unix socket server shutting down, draining {} connections",
          connections.len()
        );
        break;
      }
    }
  }

  if compio::time::timeout(drain_timeout, async {
    while connections.next().await.is_some() {}
  })
  .await
  .is_err()
  {
    for connection in connections {
      connection.cancel().await;
    }
  }

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_stopped(&path.to_string_lossy(), "unix", false)
    .await;
  Ok(())
}
