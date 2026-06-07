use std::future::Future;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
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

  /// Attach TLS material so [`Server::spawn_tls`] / [`Server::spawn_h3`] become usable.
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

  // ── HTTP family (router-driven) ──

  /// Spawn a plain HTTP/1 server.
  pub fn spawn_http(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server::serve_with_shutdown_and_config(listener, router, shutdown_fut, config).await;
    });
    handle
  }

  /// Spawn an h2c (HTTP/2 cleartext, prior knowledge) server.
  #[cfg(feature = "http2")]
  pub fn spawn_h2c(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server_h2c::serve_h2c_with_shutdown_and_config(listener, router, shutdown_fut, config)
        .await;
    });
    handle
  }

  /// Spawn a TLS server. Requires that the builder was given a [`TlsCert`].
  ///
  /// Dispatches on the [`TlsCert`] variant: `PemPaths` keeps the legacy
  /// path-loaded fast path; `Der` and `Resolver` (and any client-auth/mTLS
  /// configuration) go through [`crate::build_rustls_server_config`].
  #[cfg(feature = "tls")]
  pub fn spawn_tls(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("Server::spawn_tls requires a TlsCert (use builder().tls(...))");
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    let alpn = tls_alpn_for_tcp();
    spawn_done(handle.done.clone(), async move {
      // Plain `PemPaths` without mTLS keeps the no-overhead path-based loader;
      // every other variant goes through the rustls-config helper.
      if let TlsCert::PemPaths {
        cert_path,
        key_path,
        client_auth: None,
      } = &tls
      {
        crate::server_tls::serve_tls_with_shutdown_and_config(
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
          tracing::error!("Server::spawn_tls: failed to build rustls config: {e}");
          return;
        }
      };
      crate::server_tls::serve_tls_with_rustls_config_and_shutdown(
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

  /// Spawn an HTTP/3 (QUIC) server. Binds to `addr` internally; takes TLS
  /// from the builder. Requires the `http3` feature.
  #[cfg(feature = "http3")]
  pub fn spawn_h3(&self, addr: impl Into<String>, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("Server::spawn_h3 requires a TlsCert (use builder().tls(...))");
    let addr = addr.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      if let TlsCert::PemPaths {
        cert_path,
        key_path,
        client_auth: None,
      } = &tls
      {
        crate::server_h3::serve_h3_with_shutdown_and_config(
          router,
          &addr,
          Some(cert_path.as_str()),
          Some(key_path.as_str()),
          shutdown_fut,
          config,
        )
        .await;
        return;
      }
      let rustls_cfg = match build_rustls_server_config(&tls, vec![b"h3".to_vec()]) {
        Ok(c) => c,
        Err(e) => {
          tracing::error!("Server::spawn_h3: failed to build rustls config: {e}");
          return;
        }
      };
      crate::server_h3::serve_h3_with_rustls_config_and_shutdown(
        router,
        &addr,
        rustls_cfg,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP-over-Unix-socket server.
  #[cfg(unix)]
  pub fn spawn_unix_http(&self, path: impl Into<PathBuf>, router: Router) -> ServerHandle {
    let path = path.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server_unix::serve_unix_http_with_shutdown_and_config(
        path,
        router,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP server bound to a Linux vsock `(cid, port)` pair. Requires
  /// the `vsock` feature and Linux.
  #[cfg(all(target_os = "linux", feature = "vsock"))]
  pub fn spawn_vsock_http(&self, cid: u32, port: u32, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::server_vsock::serve_vsock_http_with_shutdown_and_config(
        cid,
        port,
        router,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  /// Spawn an HTTP server fronted by PROXY-protocol parsing.
  pub fn spawn_proxy_protocol(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      crate::proxy_protocol::serve_http_with_proxy_protocol_shutdown_and_config(
        listener,
        router,
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }

  // ── Raw transports (handler-driven, no router) ──

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
    let addr = addr.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    spawn_done(handle.done.clone(), async move {
      if let Err(e) = crate::server_tcp::serve_tcp_with_shutdown(&addr, handler, shutdown_fut).await
      {
        tracing::error!("raw TCP server error: {e}");
      }
    });
    handle
  }

  /// Spawn a raw UDP server. The handler receives each datagram.
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
    let addr = addr.into();
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    spawn_done(handle.done.clone(), async move {
      if let Err(e) = crate::server_udp::serve_udp_with_shutdown(&addr, handler, shutdown_fut).await
      {
        tracing::error!("raw UDP server error: {e}");
      }
    });
    handle
  }
}
