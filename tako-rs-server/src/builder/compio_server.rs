use tako_rs_core::router::Router;

use super::handle::ServerHandle;
use super::spawn::make_handle;
use super::spawn::spawn_done_compio;
#[cfg(feature = "compio-tls")]
use super::spawn::tls_alpn_for_tcp;
#[cfg(feature = "compio-tls")]
use super::tls_cert::TlsCert;
#[cfg(feature = "compio-tls")]
use super::tls_cert::build_rustls_server_config;
use crate::ServerConfig;

/// Fluent constructor for the compio-runtime [`CompioServer`].
#[derive(Debug, Default, Clone)]
pub struct CompioServerBuilder {
  config: ServerConfig,
  // Mirrors the gating on `CompioServer.tls` — only available when the
  // `compio-tls` feature is on so non-TLS compio builds stay warning-clean.
  #[cfg(feature = "compio-tls")]
  tls: Option<TlsCert>,
}

impl CompioServerBuilder {
  /// Override the [`ServerConfig`].
  #[must_use]
  pub fn config(mut self, config: ServerConfig) -> Self {
    self.config = config;
    self
  }

  /// Attach TLS material so [`CompioServer::spawn_tls`] becomes usable.
  #[cfg(feature = "compio-tls")]
  #[must_use]
  pub fn tls(mut self, cert: TlsCert) -> Self {
    self.tls = Some(cert);
    self
  }

  /// Finalize and produce the [`CompioServer`].
  pub fn build(self) -> CompioServer {
    CompioServer {
      config: self.config,
      #[cfg(feature = "compio-tls")]
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
  // Only consumed by the `compio-tls` impl blocks below. Marking the field
  // `cfg`-gated on the feature instead of `#[allow(dead_code)]` keeps the
  // struct layout minimal in non-TLS compio builds.
  #[cfg(feature = "compio-tls")]
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
}
