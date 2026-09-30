#[cfg(unix)]
use std::path::PathBuf;

use tako_rs_core::router::Router;

use super::handle::ServerHandle;
use super::spawn::make_handle;
use super::spawn::spawn_done_compio;
#[cfg(feature = "compio-tls")]
use super::spawn::tls_alpn_for_tcp;
#[cfg(any(feature = "compio-tls", feature = "http3"))]
use super::tls_cert::TlsCert;
#[cfg(any(feature = "compio-tls", feature = "http3"))]
use super::tls_cert::build_rustls_server_config;
use crate::ServerConfig;

/// Fluent constructor for the compio-runtime [`CompioServer`].
#[derive(Debug, Default, Clone)]
pub struct CompioServerBuilder {
  config: ServerConfig,
  // Mirrors the gating on `CompioServer.tls` — only available when TLS or
  // HTTP/3 is on so plain compio builds stay warning-clean.
  #[cfg(any(feature = "compio-tls", feature = "http3"))]
  tls: Option<TlsCert>,
}

impl CompioServerBuilder {
  /// Override the [`ServerConfig`].
  #[must_use]
  pub fn config(mut self, config: ServerConfig) -> Self {
    self.config = config;
    self
  }

  /// Attach TLS material for `spawn_tls` (`compio-tls`) or `spawn_h3` (`http3`).
  #[cfg(any(feature = "compio-tls", feature = "http3"))]
  #[must_use]
  pub fn tls(mut self, cert: TlsCert) -> Self {
    self.tls = Some(cert);
    self
  }

  /// Finalize and produce the [`CompioServer`].
  pub fn build(self) -> CompioServer {
    CompioServer {
      config: self.config,
      #[cfg(any(feature = "compio-tls", feature = "http3"))]
      tls: self.tls,
    }
  }
}

/// Compio-runtime server entry point. Construct with [`CompioServer::builder`].
///
/// Mirrors the tokio `Server` API but drives the compio runtime —
/// `io_uring` on Linux, IOCP on Windows, kqueue on macOS — under the hood.
#[derive(Debug, Clone)]
pub struct CompioServer {
  config: ServerConfig,
  // Only consumed by the TLS and HTTP/3 impl blocks below. Marking the field
  // `cfg`-gated on the features instead of `#[allow(dead_code)]` keeps the
  // struct layout minimal in plain compio builds.
  #[cfg(any(feature = "compio-tls", feature = "http3"))]
  tls: Option<TlsCert>,
}

impl CompioServer {
  /// Start a fresh fluent builder.
  #[must_use]
  pub fn builder() -> CompioServerBuilder {
    CompioServerBuilder::default()
  }

  /// Borrow the underlying [`ServerConfig`].
  #[inline]
  pub fn config(&self) -> &ServerConfig {
    &self.config
  }

  /// Spawns HTTP/1. Startup failures are returned by `result()`.
  pub fn spawn_http(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_http(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates the router and listener before starting HTTP/1.
  pub fn try_spawn_http(
    &self,
    listener: compio::net::TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done_compio(
      &handle,
      crate::server_compio::run(listener, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns HTTP/2 prior knowledge (h2c) over cleartext TCP.
  #[cfg(feature = "http2")]
  pub fn spawn_h2c(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_h2c(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates the router and listener before starting h2c.
  #[cfg(feature = "http2")]
  pub fn try_spawn_h2c(
    &self,
    listener: compio::net::TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done_compio(
      &handle,
      crate::server_compio::run_h2c(listener, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns HTTP/1 behind a PROXY protocol v1/v2 header.
  #[cfg(feature = "proxy-protocol")]
  pub fn spawn_proxy_protocol(
    &self,
    listener: compio::net::TcpListener,
    router: Router,
  ) -> ServerHandle {
    self
      .try_spawn_proxy_protocol(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates the router and listener before accepting PROXY connections.
  #[cfg(feature = "proxy-protocol")]
  pub fn try_spawn_proxy_protocol(
    &self,
    listener: compio::net::TcpListener,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(listener.local_addr()?));
    spawn_done_compio(
      &handle,
      crate::server_compio::run_proxy_protocol(listener, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }

  /// Spawns HTTP/1 on a Unix domain socket; bind failures are returned by
  /// `result()`.
  #[cfg(unix)]
  pub fn spawn_unix_http(&self, path: impl Into<PathBuf>, router: Router) -> ServerHandle {
    let path = path.into();
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done_compio(&handle, async move {
      let listener = crate::server_compio::unix::UnixSocketListener::bind(path).await?;
      crate::server_compio::run_unix(listener, router, Some(signal), config).await
    });
    handle
  }

  /// Binds the Unix socket and validates plugins before starting HTTP/1.
  #[cfg(unix)]
  pub async fn try_spawn_unix_http(
    &self,
    path: impl Into<PathBuf>,
    router: Router,
  ) -> Result<ServerHandle, tako_rs_core::types::BoxError> {
    #[cfg(feature = "plugins")]
    router.setup_plugins_once()?;
    let listener = crate::server_compio::unix::UnixSocketListener::bind(path.into()).await?;
    let config = self.config.clone();
    let (handle, signal) = make_handle(config.drain_timeout, None);
    spawn_done_compio(
      &handle,
      crate::server_compio::run_unix(listener, router, Some(signal), config),
    );
    Ok(handle)
  }

  /// Spawns TLS HTTP. Startup failures are returned by `result()`.
  #[cfg(feature = "compio-tls")]
  pub fn spawn_tls(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    self
      .try_spawn_tls(listener, router)
      .unwrap_or_else(|error| super::spawn::failed_handle(error, self.config.drain_timeout))
  }

  /// Validates certificates, plugins, and the listener before starting TLS.
  #[cfg(feature = "compio-tls")]
  pub fn try_spawn_tls(
    &self,
    listener: compio::net::TcpListener,
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
    spawn_done_compio(
      &handle,
      crate::server_tls_compio::run_with_config(
        listener,
        router,
        tls,
        Some(signal),
        self.config.clone(),
      ),
    );
    Ok(handle)
  }
  /// Spawns HTTP/3 over QUIC; startup failures are returned by `result()`.
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
    let endpoint = crate::server_compio::h3::bind_endpoint(&addr.into(), &tls, &self.config)?;
    let (handle, signal) = make_handle(self.config.drain_timeout, Some(endpoint.local_addr()?));
    spawn_done_compio(
      &handle,
      crate::server_compio::h3::run_endpoint(endpoint, router, Some(signal), self.config.clone()),
    );
    Ok(handle)
  }
}
