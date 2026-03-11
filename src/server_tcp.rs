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
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!
//! # async fn example() -> std::io::Result<()> {
//! serve_tcp("0.0.0.0:9001", |mut stream, addr| {
//!     Box::pin(async move {
//!         let mut buf = vec![0u8; 1024];
//!         loop {
//!             let n = stream.read(&mut buf).await?;
//!             if n == 0 { break; }
//!             stream.write_all(&buf[..n]).await?; // echo
//!         }
//!         Ok(())
//!     })
//! }).await?;
//! # Ok(())
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
/// In-flight connections are drained with a 30 second timeout.
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
  let listener = tokio::net::TcpListener::bind(addr).await?;
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

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
      }
      () = &mut signal => {
        tracing::info!("TCP server shutting down, draining {} connections", join_set.len());
        break;
      }
    }
  }

  // Drain in-flight connections with timeout
  let drain_timeout = std::time::Duration::from_secs(30);
  let _ = tokio::time::timeout(drain_timeout, async {
    while join_set.join_next().await.is_some() {}
  })
  .await;

  Ok(())
}

/// Starts a raw TCP server (compio runtime).
#[cfg(feature = "compio")]
pub async fn serve_tcp<F>(addr: &str, handler: F) -> std::io::Result<()>
where
  F: Fn(
      compio::net::TcpStream,
      SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
{
  let listener = compio::net::TcpListener::bind(addr).await?;
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

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
  F: Fn(
      compio::net::TcpStream,
      SocketAddr,
    ) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  use std::sync::atomic::{AtomicUsize, Ordering};

  let listener = compio::net::TcpListener::bind(addr).await?;
  tracing::info!("TCP server listening on {}", listener.local_addr()?);

  let handler = Arc::new(handler);
  let inflight = Arc::new(AtomicUsize::new(0));
  let drain_notify = Arc::new(tokio::sync::Notify::new());

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
        let inflight = Arc::clone(&inflight);
        let drain_notify = Arc::clone(&drain_notify);

        inflight.fetch_add(1, Ordering::SeqCst);

        compio::runtime::spawn(async move {
          if let Err(e) = handler(stream, peer_addr).await {
            tracing::error!("TCP connection error from {peer_addr}: {e}");
          }
          if inflight.fetch_sub(1, Ordering::SeqCst) == 1 {
            drain_notify.notify_one();
          }
        })
        .detach();
      }
      futures_util::future::Either::Right(_) => {
        tracing::info!(
          "TCP server shutting down, draining {} connections",
          inflight.load(Ordering::SeqCst)
        );
        break;
      }
    }
  }

  // Wait for in-flight connections
  if inflight.load(Ordering::SeqCst) > 0 {
    let drain_timeout = std::time::Duration::from_secs(30);
    let _ = compio::time::timeout(drain_timeout, drain_notify.notified()).await;
  }

  Ok(())
}
