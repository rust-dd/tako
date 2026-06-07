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

  /// Spawn a compio HTTP/1 server.
  pub fn spawn_http(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done_compio(handle.done.clone(), async move {
      crate::server_compio::serve_with_shutdown_and_config(listener, router, shutdown_fut, config)
        .await;
    });
    handle
  }

  /// Spawn a compio TLS server.
  #[cfg(feature = "compio-tls")]
  pub fn spawn_tls(&self, listener: compio::net::TcpListener, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("CompioServer::spawn_tls requires a TlsCert (use builder().tls(...))");
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    let alpn = tls_alpn_for_tcp();
    spawn_done_compio(handle.done.clone(), async move {
      if let TlsCert::PemPaths {
        cert_path,
        key_path,
        client_auth: None,
      } = &tls
      {
        crate::server_tls_compio::serve_tls_with_shutdown_and_config(
          listener,
          router,
          Some(cert_path.as_str()),
          Some(key_path.as_str()),
          shutdown_fut,
          config,
        )
        .await;
        return;
      }
      let rustls_cfg = match build_rustls_server_config(&tls, alpn) {
        Ok(c) => c,
        Err(e) => {
          tracing::error!("CompioServer::spawn_tls: failed to build rustls config: {e}");
          return;
        }
      };
      crate::server_tls_compio::serve_tls_with_rustls_config_and_shutdown(
        listener,
        router,
        rustls_cfg,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }
}
