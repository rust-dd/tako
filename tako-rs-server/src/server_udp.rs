//! Raw UDP servers that hand each datagram to a handler, on Tokio or compio.
//!
//! # Examples
//!
//! ```rust,no_run
//! use tako::server_udp::serve_udp;
//!
//! # async fn example() -> std::io::Result<()> {
//! serve_udp("0.0.0.0:9000", |data, addr, socket| {
//!     Box::pin(async move {
//!         # #[cfg(not(feature = "compio"))]
//!         let _ = socket.send_to(&data, addr).await;
//!         # #[cfg(feature = "compio")]
//!         # let _ = socket.send_to(data, addr).await;
//!     })
//! }).await?;
//! # Ok(())
//! # }
//! ```

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

#[cfg(feature = "compio")]
use futures_util::FutureExt;
#[cfg(feature = "compio")]
use futures_util::StreamExt;

use crate::ServerConfig;

/// Runs `handler` for every datagram until a receive error, which it returns.
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
  serve_udp_with_config(
    addr,
    handler,
    std::future::pending(),
    ServerConfig::default(),
  )
  .await
}

/// Stops receiving once `signal` resolves and gives running handlers up to 30 seconds.
#[cfg(not(feature = "compio"))]
pub async fn serve_udp_with_shutdown<F, S>(addr: &str, handler: F, signal: S) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<tokio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  serve_udp_with_config(addr, handler, signal, ServerConfig::default()).await
}

/// Runs raw UDP with a cap on active handlers. Datagrams received at capacity are dropped.
#[cfg(not(feature = "compio"))]
pub async fn serve_udp_with_config<F, S>(
  addr: &str,
  handler: F,
  signal: S,
  config: ServerConfig,
) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<tokio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  config.validate()?;
  let socket = tokio::net::UdpSocket::bind(addr).await?;
  run_socket(socket, handler, signal, config).await
}

#[cfg(not(feature = "compio"))]
pub(crate) async fn run_socket<F, S>(
  socket: tokio::net::UdpSocket,
  handler: F,
  signal: S,
  config: ServerConfig,
) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<tokio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()> + Send>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + Send + 'static,
{
  config.validate()?;
  let socket = Arc::new(socket);
  let mut tasks = tokio::task::JoinSet::new();
  tracing::info!("UDP server listening on {}", socket.local_addr()?);

  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &socket.local_addr()?.to_string(),
    "udp",
    false,
  )
  .await;

  let handler = Arc::new(handler);
  let mut buf = vec![0u8; 65535];

  tokio::pin!(signal);

  loop {
    tokio::select! {
      biased;
      () = &mut signal => {
        tracing::info!("UDP server shutting down");
        break;
      }
      _ = tasks.join_next(), if !tasks.is_empty() => {}
      result = socket.recv_from(&mut buf) => {
        let (len, peer) = result?;
        if config.max_connections.is_some_and(|max| tasks.len() >= max) {
          continue;
        }
        let data = buf[..len].to_vec();
        let socket = Arc::clone(&socket);
        let handler = Arc::clone(&handler);

        tasks.spawn(async move {
          handler(data, peer, socket).await;
        });
      }
    }
  }

  if tokio::time::timeout(config.drain_timeout, async {
    while tasks.join_next().await.is_some() {}
  })
  .await
  .is_err()
  {
    tasks.shutdown().await;
  }
  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_stopped(
    &socket.local_addr()?.to_string(),
    "udp",
    false,
  )
  .await;
  Ok(())
}

/// Runs `handler` for every datagram until a receive error, which it returns.
#[cfg(feature = "compio")]
pub async fn serve_udp<F>(addr: &str, handler: F) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync
    + 'static,
{
  serve_udp_with_config(
    addr,
    handler,
    std::future::pending(),
    ServerConfig::default(),
  )
  .await
}

/// Stops receiving once `signal` resolves and gives running handlers up to 30 seconds.
#[cfg(feature = "compio")]
pub async fn serve_udp_with_shutdown<F, S>(addr: &str, handler: F, signal: S) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  serve_udp_with_config(addr, handler, signal, ServerConfig::default()).await
}

/// Runs raw UDP with a cap on active handlers. Datagrams received at capacity are dropped.
#[cfg(feature = "compio")]
pub async fn serve_udp_with_config<F, S>(
  addr: &str,
  handler: F,
  signal: S,
  config: ServerConfig,
) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  config.validate()?;
  let socket = compio::net::UdpSocket::bind(addr).await?;
  run_socket(socket, handler, signal, config).await
}

#[cfg(feature = "compio")]
pub(crate) async fn run_socket<F, S>(
  socket: compio::net::UdpSocket,
  handler: F,
  signal: S,
  config: ServerConfig,
) -> std::io::Result<()>
where
  F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
    + Send
    + Sync
    + 'static,
  S: Future<Output = ()> + 'static,
{
  config.validate()?;
  // compio is thread-per-core; the Arc<UdpSocket> never crosses threads.
  #[allow(clippy::arc_with_non_send_sync)]
  let socket = Arc::new(socket);
  tracing::info!("UDP server listening on {}", socket.local_addr()?);

  let mut tasks = futures_util::stream::FuturesUnordered::new();
  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_started(
    &socket.local_addr()?.to_string(),
    "udp",
    false,
  )
  .await;

  let handler = Arc::new(handler);
  let signal = std::pin::pin!(signal);

  let mut signal = signal;

  loop {
    while tasks.next().now_or_never().flatten().is_some() {}
    let buf = vec![0u8; 65535];
    let recv_fut = socket.recv_from(buf);

    let recv_fut = std::pin::pin!(recv_fut);
    match futures_util::future::select(&mut signal, recv_fut).await {
      futures_util::future::Either::Right((compio::BufResult(result, buf_out), _)) => {
        let (len, peer) = result?;
        while tasks.next().now_or_never().flatten().is_some() {}
        if config.max_connections.is_some_and(|max| tasks.len() >= max) {
          continue;
        }
        let mut buf = buf_out;
        buf.truncate(len);
        let socket = Arc::clone(&socket);
        let handler = Arc::clone(&handler);

        tasks.push(compio::runtime::spawn(async move {
          handler(buf, peer, socket).await;
        }));
      }
      futures_util::future::Either::Left(_) => {
        tracing::info!("UDP server shutting down");
        break;
      }
    }
  }

  if compio::time::timeout(config.drain_timeout, async {
    while tasks.next().await.is_some() {}
  })
  .await
  .is_err()
  {
    for task in tasks {
      task.cancel().await;
    }
  }
  #[cfg(feature = "signals")]
  tako_rs_core::signals::transport::emit_server_stopped(
    &socket.local_addr()?.to_string(),
    "udp",
    false,
  )
  .await;
  Ok(())
}
