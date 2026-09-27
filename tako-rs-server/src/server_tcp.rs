//! Raw TCP server for handling arbitrary TCP connections.
//!
//! Provides a TCP server that accepts connections and dispatches them through
//! a user-defined handler function with raw read/write access. Supports both
//! tokio and compio runtimes.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::server_tcp::serve_tcp;
//! # #[cfg(not(feature = "compio"))]
//! # async fn example() -> std::io::Result<()> {
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//! serve_tcp("0.0.0.0:9001", |mut stream, _addr| {
//!     Box::pin(async move {
//!         let mut buf = vec![0u8; 1024];
//!         loop {
//!             let n = stream.read(&mut buf).await?;
//!             if n == 0 { break; }
//!             stream.write_all(&buf[..n]).await?;
//!         }
//!         Ok(())
//!     })
//! }).await?;
//! # Ok(())
//! # }
//! # #[cfg(feature = "compio")]
//! # async fn example() -> std::io::Result<()> {
//! # use compio::io::{AsyncRead, AsyncWriteExt};
//! # serve_tcp("0.0.0.0:9001", |mut stream, _addr| Box::pin(async move {
//! #     let mut buf = vec![0u8; 1024];
//! #     loop {
//! #         let compio::BufResult(result, mut returned) = stream.read(buf).await;
//! #         let n = result?;
//! #         if n == 0 { break; }
//! #         returned.truncate(n);
//! #         let compio::BufResult(result, mut returned) = stream.write_all(returned).await;
//! #         result?;
//! #         returned.resize(1024, 0);
//! #         buf = returned;
//! #     }
//! #     Ok(())
//! # })).await
//! # }
//! ```

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

/// Starts a raw TCP server (tokio runtime).
///
/// Each accepted connection is dispatched to the handler with the TCP stream
/// and the peer's socket address.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::server_tcp::serve_tcp;
/// use tokio::io::{AsyncReadExt, AsyncWriteExt};
///
/// # async fn example() -> std::io::Result<()> {
/// serve_tcp("0.0.0.0:9001", |mut stream, addr| {
///     Box::pin(async move {
///         println!("Connection from {addr}");
///         let mut buf = vec![0u8; 4096];
///         let n = stream.read(&mut buf).await?;
///         stream.write_all(&buf[..n]).await?;
///         Ok(())
///     })
/// }).await?;
/// # Ok(())
/// # }
/// ```
#[cfg(not(feature = "compio"))]
pub async fn serve_tcp<F>(addr: &str, handler: F) -> std::io::Result<()>
where
  F: Fn(
      tokio::net::TcpStream,
      SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
    + Send
    + Sync
    + 'static,
{
  let listener = tokio::net::TcpListener::bind(addr).await?;
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;

  let handler = Arc::new(handler);

  loop {
    let (stream, peer_addr) = listener.accept().await?;
    let _ = stream.set_nodelay(true);
    let handler = Arc::clone(&handler);

    tokio::spawn(async move {
      if let Err(e) = handler(stream, peer_addr).await {
        tracing::error!("TCP connection error from {peer_addr}: {e}");
      }
    });
  }
}

/// Starts a raw TCP server with a shutdown signal (tokio runtime).
///
/// The server stops accepting new connections when the shutdown signal completes.
/// In-flight connections are drained with a 30 second timeout. Use
/// [`serve_tcp_with_shutdown_and_drain`] to override this bound.
#[cfg(not(feature = "compio"))]
pub async fn serve_tcp_with_shutdown<F, S>(addr: &str, handler: F, signal: S) -> std::io::Result<()>
where
  F: Fn(
      tokio::net::TcpStream,
      SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  serve_tcp_with_shutdown_and_drain(addr, handler, signal, std::time::Duration::from_secs(30)).await
}

/// Same as [`serve_tcp_with_shutdown`] but with an explicit drain timeout.
#[cfg(not(feature = "compio"))]
pub async fn serve_tcp_with_shutdown_and_drain<F, S>(
  addr: &str,
  handler: F,
  signal: S,
  drain_timeout: std::time::Duration,
) -> std::io::Result<()>
where
  F: Fn(
      tokio::net::TcpStream,
      SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  let listener = tokio::net::TcpListener::bind(addr).await?;
  run_listener(listener, handler, signal, drain_timeout).await
}

#[cfg(not(feature = "compio"))]
pub(crate) async fn run_listener<F, S>(
  listener: tokio::net::TcpListener,
  handler: F,
  signal: S,
  drain_timeout: std::time::Duration,
) -> std::io::Result<()>
where
  F: Fn(
      tokio::net::TcpStream,
      SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;

  let handler = Arc::new(handler);
  let mut join_set = tokio::task::JoinSet::new();

  tokio::pin!(signal);

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (stream, peer_addr) = result?;
        let _ = stream.set_nodelay(true);
        let handler = Arc::clone(&handler);

        join_set.spawn(async move {
          if let Err(e) = handler(stream, peer_addr).await {
            tracing::error!("TCP connection error from {peer_addr}: {e}");
          }
        });
        while join_set.try_join_next().is_some() {}
      }
      () = &mut signal => {
        tracing::info!("TCP server shutting down, draining {} connections", join_set.len());
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
  tako_rs_core::signals::transport::emit_server_stopped(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;
  Ok(())
}

/// Starts a raw TCP server (compio runtime).
#[cfg(feature = "compio")]
pub async fn serve_tcp<F>(addr: &str, handler: F) -> std::io::Result<()>
where
  F: Fn(compio::net::TcpStream, SocketAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
{
  let listener = compio::net::TcpListener::bind(addr).await?;
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;

  let handler = Arc::new(handler);

  loop {
    let (stream, peer_addr) = listener.accept().await?;
    let _ = stream.set_nodelay(true);
    let handler = Arc::clone(&handler);

    compio::runtime::spawn(async move {
      if let Err(e) = handler(stream, peer_addr).await {
        tracing::error!("TCP connection error from {peer_addr}: {e}");
      }
    })
    .detach();
  }
}

/// Starts a raw TCP server with a shutdown signal (compio runtime).
#[cfg(feature = "compio")]
pub async fn serve_tcp_with_shutdown<F, S>(addr: &str, handler: F, signal: S) -> std::io::Result<()>
where
  F: Fn(compio::net::TcpStream, SocketAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  serve_tcp_with_shutdown_and_drain(addr, handler, signal, std::time::Duration::from_secs(30)).await
}

/// Same as [`serve_tcp_with_shutdown`] but with an explicit drain timeout.
#[cfg(feature = "compio")]
pub async fn serve_tcp_with_shutdown_and_drain<F, S>(
  addr: &str,
  handler: F,
  signal: S,
  drain_timeout: std::time::Duration,
) -> std::io::Result<()>
where
  F: Fn(compio::net::TcpStream, SocketAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  use futures_util::FutureExt;
  use futures_util::StreamExt;

  let listener = compio::net::TcpListener::bind(addr).await?;
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;

  let handler = Arc::new(handler);
  let mut connections = futures_util::stream::FuturesUnordered::new();

  let signal = std::pin::pin!(signal);
  let mut signal = signal;

  loop {
    let accept_fut = listener.accept();
    let accept_fut = std::pin::pin!(accept_fut);

    match futures_util::future::select(accept_fut, &mut signal).await {
      futures_util::future::Either::Left((result, _)) => {
        let (stream, peer_addr) = result?;
        let _ = stream.set_nodelay(true);
        let handler = Arc::clone(&handler);
        connections.push(compio::runtime::spawn(async move {
          if let Err(e) = handler(stream, peer_addr).await {
            tracing::error!("TCP connection error from {peer_addr}: {e}");
          }
        }));
        while connections.next().now_or_never().flatten().is_some() {}
      }
      futures_util::future::Either::Right(_) => {
        tracing::info!(
          "TCP server shutting down, draining {} connections",
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
  tako_rs_core::signals::transport::emit_server_stopped(
    &listener.local_addr()?.to_string(),
    "tcp",
    false,
  )
  .await;
  Ok(())
}
