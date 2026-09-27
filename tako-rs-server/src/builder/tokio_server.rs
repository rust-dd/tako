use std::future::Future;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
#[cfg(feature = "udp")]
use std::sync::Arc;

use tako_rs_core::router::Router;
use tokio::net::TcpListener;

use super::handle::ServerHandle;
use super::spawn::make_handle;
use super::spawn::spawn_done;
#[cfg(feature = "tls")]
use super::spawn::tls_alpn_for_tcp;
use super::tls_cert::TlsCert;
#[cfg(feature = "tls")]
use super::tls_cert::build_rustls_server_config;
use crate::ServerConfig;

/// Fluent constructor for the tokio-runtime [`Server`].
#[derive(Debug, Default, Clone)]
pub struct ServerBuilder {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

impl ServerBuilder {
  /// Override the [`ServerConfig`] (drain timeout, h2 caps, `max_connections`, …).
  #[must_use]
  pub fn config(mut self, config: ServerConfig) -> Self {
    self.config = config;
    self
  }

  /// Attach TLS material for `spawn_tls` (`tls`) or `spawn_h3` (`http3`).
  #[must_use]
  pub fn tls(mut self, cert: TlsCert) -> Self {
    self.tls = Some(cert);
    self
  }

  /// Finalize and produce the [`Server`].
  pub fn build(self) -> Server {
    Server {
      config: self.config,
      tls: self.tls,
    }
  }
}

/// Tokio-runtime server entry point. Construct with [`Server::builder`].
#[derive(Debug, Clone)]
pub struct Server {
  config: ServerConfig,
  // Read only by the `tls` / `http3` cfg-gated spawn methods; the field is
  // always present so the builder API surface stays stable across feature
  // combinations.
  #[cfg_attr(not(any(feature = "tls", feature = "http3")), allow(dead_code))]
  tls: Option<TlsCert>,
}

impl Server {
  /// Start a fresh fluent builder.
  #[must_use]
  pub fn builder() -> ServerBuilder {
    ServerBuilder::default()
  }

  /// Borrow the underlying [`ServerConfig`].
  #[inline]
  pub fn config(&self) -> &ServerConfig {
    &self.config
  }

  /// Spawns HTTP/1; inspect [`ServerHandle::result`] for startup failures.
  pub fn spawn_http(&self, listener: TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_http(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates the router and listener before starting HTTP/1.
  pub fn try_spawn_http(
    &self,
    listener: TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done(
      &handle,
      crate::server::run(listener, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns HTTP/2 prior knowledge over TCP.
  #[cfg(feature = "http2")]
  pub fn spawn_h2c(&self, listener: TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_h2c(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates the router and listener before starting h2c.
  #[cfg(feature = "http2")]
  pub fn try_spawn_h2c(
    &self,
    listener: TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done(
      &handle,
      crate::server_h2c::run(listener, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns TLS HTTP; missing or invalid certificates are returned by `result()`.
  #[cfg(feature = "tls")]
  pub fn spawn_tls(&self, listener: TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_tls(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates certificates, plugin setup, and the listener before starting TLS.
  #[cfg(feature = "tls")]
  pub fn try_spawn_tls(
    &self,
    listener: TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    let tls = self
      .tls
      .as_ref()
      .ok_or("TLS certificate configuration is required")?;
    let tls = build_rustls_server_config(tls, tls_alpn_for_tcp())?;
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done(
      &handle,
      crate::server_tls::run_with_config(listener, router, tls, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns HTTP/3; startup failures are returned by `result()`.
  #[cfg(feature = "http3")]
  pub fn spawn_h3(&self, addr: impl Into<String>, router: Router) -> ServerHandle {
    self
      .try_spawn_h3(addr, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates TLS and binds the QUIC endpoint before starting HTTP/3.
  #[cfg(feature = "http3")]
  pub fn try_spawn_h3(
    &self,
    addr: impl Into<String>,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    let tls = self
      .tls
      .as_ref()
      .ok_or("TLS certificate configuration is required")?;
    let tls = build_rustls_server_config(tls, vec![b"h3".to_vec()])?;
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let endpoint = crate::server_h3::run::bind_endpoint(&addr.into(), tls, &self.config)?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(endpoint.local_addr()?));
    spawn_done(
      &handle,
      crate::server_h3::run::run_endpoint(endpoint, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns HTTP over a Unix socket. Bind failures are returned by `result()`.
  #[cfg(unix)]
  pub fn spawn_unix_http(&self, path: impl Into<PathBuf>, router: Router) -> ServerHandle {
    let path = path.into();
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done(&handle, async move {
      crate::server_unix::http::run_http(&path, router, Some(signal), config).await
    });
    handle
  }

  /// Binds the Unix socket and validates plugins before starting HTTP.
  #[cfg(unix)]
  pub async fn try_spawn_unix_http(
    &self,
    path: impl Into<PathBuf>,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    let path = path.into();
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let listener = crate::server_unix::listener::bind_unix_listener(&path).await?;
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done(&handle, async move {
      crate::server_unix::http::run_listener(listener, &path, router, Some(signal), config).await
    });
    Ok(handle)
  }

  /// Spawns HTTP on a Linux vsock address.
  #[cfg(all(target_os = "linux", feature = "vsock"))]
  pub fn spawn_vsock_http(&self, cid: u32, port: u32, router: Router) -> ServerHandle {
    self
      .try_spawn_vsock_http(cid, port, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Binds the vsock listener before accepting HTTP connections.
  #[cfg(all(target_os = "linux", feature = "vsock"))]
  pub fn try_spawn_vsock_http(
    &self,
    cid: u32,
    port: u32,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let listener = tokio_vsock::VsockListener::bind(tokio_vsock::VsockAddr::new(cid, port))?;
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done(
      &handle,
      crate::server_vsock::run_listener(listener, cid, port, router, Some(signal), config),
    );
    Ok(handle)
  }

  /// Spawns HTTP behind a PROXY-protocol listener.
  #[cfg(feature = "proxy-protocol")]
  pub fn spawn_proxy_protocol(&self, listener: TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_proxy_protocol(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates the router and listener before accepting PROXY connections.
  #[cfg(feature = "proxy-protocol")]
  pub fn try_spawn_proxy_protocol(
    &self,
    listener: TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done(
      &handle,
      crate::proxy_protocol::listener::run_proxy_http(
        listener,
        router,
        Some(signal),
        self.config.clone(),
      ),
    );
    Ok(handle)
  }

  /// Spawn a raw TCP server. The handler receives each accepted stream.
  pub fn spawn_tcp_raw<F>(&self, addr: impl Into<String>, handler: F) -> ServerHandle
  where
    F: Fn(
        tokio::net::TcpStream,
        std::net::SocketAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
      + Send
      + Sync
      + 'static,
  {
    self
      .try_spawn_tcp_raw(addr, handler)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Binds the socket before starting the raw transport.
  pub fn try_spawn_tcp_raw<F>(
    &self,
    addr: impl Into<String>,
    handler: F,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError>
  where
    F: Fn(
        tokio::net::TcpStream,
        std::net::SocketAddr,
      ) -> Pin<Box<dyn Future<Output = std::io::Result<()>> + Send>>
      + Send
      + Sync
      + 'static,
  {
    let bound = std::net::TcpListener::bind(addr.into())?;
    bound.set_nonblocking(true)?;
    let listener = tokio::net::TcpListener::from_std(bound)?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    let drain_timeout = self.config.drain_timeout;
    spawn_done(&handle, async move {
      crate::server_tcp::run_listener(listener, handler, signal, drain_timeout)
        .await
        .map_err(Into::into)
    });
    Ok(handle)
  }

  /// Spawn a raw UDP server. The handler receives each datagram.
  #[cfg(feature = "udp")]
  pub fn spawn_udp_raw<F>(&self, addr: impl Into<String>, handler: F) -> ServerHandle
  where
    F: Fn(
        Vec<u8>,
        std::net::SocketAddr,
        Arc<tokio::net::UdpSocket>,
      ) -> Pin<Box<dyn Future<Output = ()> + Send>>
      + Send
      + Sync
      + 'static,
  {
    self
      .try_spawn_udp_raw(addr, handler)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Binds the socket before starting the raw transport.
  #[cfg(feature = "udp")]
  pub fn try_spawn_udp_raw<F>(
    &self,
    addr: impl Into<String>,
    handler: F,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError>
  where
    F: Fn(
        Vec<u8>,
        std::net::SocketAddr,
        Arc<tokio::net::UdpSocket>,
      ) -> Pin<Box<dyn Future<Output = ()> + Send>>
      + Send
      + Sync
      + 'static,
  {
    let bound = std::net::UdpSocket::bind(addr.into())?;
    bound.set_nonblocking(true)?;
    let socket = tokio::net::UdpSocket::from_std(bound)?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(socket.local_addr()?));
    let drain_timeout = self.config.drain_timeout;
    spawn_done(&handle, async move {
      crate::server_udp::run_socket(socket, handler, signal, drain_timeout)
        .await
        .map_err(Into::into)
    });
    Ok(handle)
  }
}
