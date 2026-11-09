use std::future::Future;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
#[cfg(feature = "udp")]
use std::sync::Arc;

use super::CompioServer;
use crate::ServerHandle;
use crate::builder::spawn::failed_handle;
use crate::builder::spawn::make_handle;
use crate::builder::spawn::spawn_done_compio;

impl CompioServer {
  /// Spawns raw TCP with the configured connection cap and shutdown deadline.
  pub fn spawn_tcp_raw<F>(&self, addr: impl Into<String>, handler: F) -> ServerHandle
  where
    F: Fn(compio::net::TcpStream, SocketAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
      + Send
      + Sync
      + 'static,
  {
    self
      .try_spawn_tcp_raw(addr, handler)
      .unwrap_or_else(|error| failed_handle(error, self.config.drain_timeout))
  }

  /// Validates configuration and binds the raw TCP listener before starting tasks.
  pub fn try_spawn_tcp_raw<F>(
    &self,
    addr: impl Into<String>,
    handler: F,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError>
  where
    F: Fn(compio::net::TcpStream, SocketAddr) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
      + Send
      + Sync
      + 'static,
  {
    self.config.validate()?;
    let listener = compio::net::TcpListener::from_std(std::net::TcpListener::bind(addr.into())?)?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    let config = self.config.clone();
    spawn_done_compio(&handle, async move {
      crate::server_tcp::run_listener(listener, handler, signal, config)
        .await
        .map_err(Into::into)
    });
    Ok(handle)
  }

  /// Spawns raw UDP; datagrams received at the configured handler cap are dropped.
  #[cfg(feature = "udp")]
  pub fn spawn_udp_raw<F>(&self, addr: impl Into<String>, handler: F) -> ServerHandle
  where
    F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
      + Send
      + Sync
      + 'static,
  {
    self
      .try_spawn_udp_raw(addr, handler)
      .unwrap_or_else(|error| failed_handle(error, self.config.drain_timeout))
  }

  /// Validates configuration and binds the raw UDP socket before starting tasks.
  #[cfg(feature = "udp")]
  pub fn try_spawn_udp_raw<F>(
    &self,
    addr: impl Into<String>,
    handler: F,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError>
  where
    F: Fn(Vec<u8>, SocketAddr, Arc<compio::net::UdpSocket>) -> Pin<Box<dyn Future<Output = ()>>>
      + Send
      + Sync
      + 'static,
  {
    self.config.validate()?;
    let socket = compio::net::UdpSocket::from_std(std::net::UdpSocket::bind(addr.into())?)?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(socket.local_addr()?));
    let config = self.config.clone();
    spawn_done_compio(&handle, async move {
      crate::server_udp::run_socket(socket, handler, signal, config)
        .await
        .map_err(Into::into)
    });
    Ok(handle)
  }

  /// Spawns raw Unix sockets; startup errors are returned by [`ServerHandle::result`].
  #[cfg(unix)]
  pub fn spawn_unix_raw<F>(&self, path: impl Into<PathBuf>, handler: F) -> ServerHandle
  where
    F: Fn(
        compio::net::UnixStream,
        tako_rs_core::conn_info::UnixPeerAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
      + Send
      + Sync
      + 'static,
  {
    if let Err(error) = self.config.validate() {
      return failed_handle(error.into(), self.config.drain_timeout);
    }
    let path = path.into();
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done_compio(&handle, async move {
      crate::server_unix::serve_unix_with_config(path, handler, signal, config)
        .await
        .map_err(Into::into)
    });
    handle
  }

  /// Validates configuration and binds the raw Unix listener before starting tasks.
  #[cfg(unix)]
  pub async fn try_spawn_unix_raw<F>(
    &self,
    path: impl Into<PathBuf>,
    handler: F,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError>
  where
    F: Fn(
        compio::net::UnixStream,
        tako_rs_core::conn_info::UnixPeerAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>>>>
      + Send
      + Sync
      + 'static,
  {
    self.config.validate()?;
    let path = path.into();
    let listener = crate::server_compio::unix::bind_listener(&path).await?;
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done_compio(&handle, async move {
      crate::server_unix::run_listener(listener, &path, handler, signal, config)
        .await
        .map_err(Into::into)
    });
    Ok(handle)
  }
}
