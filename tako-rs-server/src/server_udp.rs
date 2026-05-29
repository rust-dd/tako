//! UDP datagram server for handling raw UDP packets.
//!
//! Provides a simple UDP server that receives datagrams and dispatches them
//! through a user-defined handler function. Supports both tokio and compio runtimes.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::server_udp::serve_udp;
//!
//! # async fn example() -> std::io::Result<()> {
//! serve_udp("0.0.0.0:9000", |data, addr, socket| {
//!     Box::pin(async move {
//!         // Echo back
//!         let _ = socket.send_to(&data, addr).await;
//!     })
//! }).await?;
//! # Ok(())
//! # }
//! ```

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

/// Handler function type for UDP datagrams.
///
/// Receives the raw bytes, the sender's address, and a reference to the socket
/// for sending responses.
#[cfg(not(feature = "compio"))]
pub type UdpHandler = Arc<
  dyn Fn(Vec<u8>, SocketAddr, Arc<tokio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync,
>;

#[cfg(feature = "compio")]
pub type UdpHandler = Arc<
  dyn Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync,
>;

/// Starts a UDP server that listens on the given address.
///
/// The handler is called for each received datagram with the raw bytes,
/// the sender's address, and a reference to the socket.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::server_udp::serve_udp;
/// use std::sync::Arc;
///
/// # async fn example() -> std::io::Result<()> {
/// serve_udp("0.0.0.0:9000", |data, addr, socket| {
///     Box::pin(async move {
///         println!("Received {} bytes from {}", data.len(), addr);
///         let _ = socket.send_to(&data, addr).await;
///     })
/// }).await?;
/// # Ok(())
/// # }
/// ```
#[cfg(not(feature = "compio"))]
pub async fn serve_udp<F>(addr: &str, handler: F) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<tokio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
{
  let socket = Arc::new(tokio::net::UdpSocket::bind(addr).await?);
  tracing::info!("UDP server listening on {}", socket.local_addr()?);

  let handler = Arc::new(handler);
  let mut buf = vec![0u8; 65535];

  loop {
    let (len, peer) = socket.recv_from(&mut buf).await?;
    let data = buf[..len].to_vec();
    let socket = Arc::clone(&socket);
    let handler = Arc::clone(&handler);

    tokio::spawn(async move {
      handler(data, peer, socket).await;
    });
  }
}

/// Starts a UDP server with a shutdown signal.
///
/// The server stops accepting new datagrams when the shutdown signal completes.
#[cfg(not(feature = "compio"))]
pub async fn serve_udp_with_shutdown<F, S>(addr: &str, handler: F, signal: S) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<tokio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  let socket = Arc::new(tokio::net::UdpSocket::bind(addr).await?);
  tracing::info!("UDP server listening on {}", socket.local_addr()?);

  let handler = Arc::new(handler);
  let mut buf = vec![0u8; 65535];

  tokio::pin!(signal);

  loop {
    tokio::select! {
      result = socket.recv_from(&mut buf) => {
        let (len, peer) = result?;
        let data = buf[..len].to_vec();
        let socket = Arc::clone(&socket);
        let handler = Arc::clone(&handler);

        tokio::spawn(async move {
          handler(data, peer, socket).await;
        });
      }
      () = &mut signal => {
        tracing::info!("UDP server shutting down");
        break;
      }
    }
  }

  Ok(())
}

/// Starts a UDP server (compio runtime).
#[cfg(feature = "compio")]
pub async fn serve_udp<F>(addr: &str, handler: F) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync
    + 'static,
{
  // compio is thread-per-core; the Arc<UdpSocket> never crosses threads.
  #[allow(clippy::arc_with_non_send_sync)]
  let socket = Arc::new(compio::net::UdpSocket::bind(addr).await?);
  tracing::info!("UDP server listening on {}", socket.local_addr()?);

  let handler = Arc::new(handler);

  loop {
    let buf = vec![0u8; 65535];
    let compio::BufResult(result, mut buf) = socket.recv_from(buf).await;
    let (len, peer) = result?;
    buf.truncate(len);
    let socket = Arc::clone(&socket);
    let handler = Arc::clone(&handler);

    compio::runtime::spawn(async move {
      handler(buf, peer, socket).await;
    })
    .detach();
  }
}

/// Starts a UDP server with a shutdown signal (compio runtime).
#[cfg(feature = "compio")]
pub async fn serve_udp_with_shutdown<F, S>(addr: &str, handler: F, signal: S) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  // compio is thread-per-core; the Arc<UdpSocket> never crosses threads.
  #[allow(clippy::arc_with_non_send_sync)]
  let socket = Arc::new(compio::net::UdpSocket::bind(addr).await?);
  tracing::info!("UDP server listening on {}", socket.local_addr()?);

  let handler = Arc::new(handler);
  let signal = std::pin::pin!(signal);

  // compio uses futures_util::future::select for cancellation
  let mut signal = signal;

  loop {
    let buf = vec![0u8; 65535];
    let recv_fut = socket.recv_from(buf);

    let recv_fut = std::pin::pin!(recv_fut);
    match futures_util::future::select(recv_fut, &mut signal).await {
      futures_util::future::Either::Left((compio::BufResult(result, buf_out), _)) => {
        let (len, peer) = result?;
        let mut buf = buf_out;
        buf.truncate(len);
        let socket = Arc::clone(&socket);
        let handler = Arc::clone(&handler);

        compio::runtime::spawn(async move {
          handler(buf, peer, socket).await;
        })
        .detach();
      }
      futures_util::future::Either::Right(_) => {
        tracing::info!("UDP server shutting down");
        break;
      }
    }
  }

  Ok(())
}
