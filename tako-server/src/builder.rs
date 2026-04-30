//! Unified [`Server`] / [`CompioServer`] builder fronting every Tako transport.
//!
//! The direct `serve_*` / `serve_*_with_shutdown` / `*_with_config` functions
//! still exist and keep working. This module is an additive convenience layer:
//! pick a transport via `spawn_*`, hand it a [`crate::ServerConfig`], and get
//! back a [`ServerHandle`] that owns a shutdown trigger.
//!
//! The handle itself is runtime-agnostic — both [`Server`] (tokio) and
//! [`CompioServer`] (cfg `compio`) return the same [`ServerHandle`] type.
//! Internally each `spawn_*` wraps the underlying `serve_*` future so that
//! when it returns, a `done` [`Notify`] is signalled. [`ServerHandle::join`]
//! awaits that notify; [`ServerHandle::shutdown`] triggers the shutdown
//! signal and then awaits the same `done`.
//!
//! No additional allocation or atomic swap is introduced on the per-connection
//! / per-request hot path — the spawn wrapper is a single async block over the
//! underlying `serve_*_with_shutdown_and_config` call.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Notify;

#[cfg(not(feature = "compio"))]
use std::pin::Pin;

#[cfg(not(feature = "compio"))]
use std::path::PathBuf;
#[cfg(not(feature = "compio"))]
use tokio::net::TcpListener;

use tako_core::router::Router;

use crate::ServerConfig;

// ───────────────────────── shared handle ──────────────────────────

/// Background-task handle returned by every `spawn_*` method.
///
/// Drop semantics: dropping the handle does **not** stop the server. Call
/// [`ServerHandle::shutdown`] (or [`ServerHandle::trigger`] + `.join().await`)
/// so the drain logic in the underlying `serve_*_with_shutdown` runs.
///
/// Runtime-agnostic — the `done` signal is fired by an `async` wrapper around
/// the underlying `serve_*` future, so the same `ServerHandle` works whether
/// the spawned task lives on the tokio runtime or the compio runtime.
pub struct ServerHandle {
  shutdown: Arc<Notify>,
  done: Arc<Notify>,
  drain_timeout: Duration,
}

impl ServerHandle {
  /// Trigger graceful shutdown without awaiting completion.
  pub fn trigger(&self) {
    self.shutdown.notify_waiters();
  }

  /// Await the spawned task's completion (without triggering shutdown).
  ///
  /// Returns when the underlying `serve_*` future resolves — typically
  /// because [`ServerHandle::trigger`] / [`ServerHandle::shutdown`] was called
  /// or because the listener errored fatally.
  pub async fn join(&self) {
    self.done.notified().await;
  }

  /// Trigger graceful shutdown and await the drain.
  ///
  /// The `_timeout` argument is kept for API symmetry with the original
  /// builder; the actual drain bound is the `drain_timeout` on the
  /// [`ServerConfig`] that was handed to the builder, enforced inside
  /// `serve_*_with_shutdown`.
  pub async fn shutdown(self, _timeout: Duration) {
    self.shutdown.notify_waiters();
    self.done.notified().await;
  }

  /// Returns the drain timeout the underlying `serve_*` will honor.
  #[inline]
  pub fn drain_timeout(&self) -> Duration {
    self.drain_timeout
  }
}

impl std::fmt::Debug for ServerHandle {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ServerHandle")
      .field("drain_timeout", &self.drain_timeout)
      .finish_non_exhaustive()
  }
}

/// Convenience: await `signal_a` *or* `signal_b`, whichever fires first.
pub async fn either<A, B>(a: A, b: B)
where
  A: Future<Output = ()>,
  B: Future<Output = ()>,
{
  use futures_util::future::Either;
  let a = std::pin::pin!(a);
  let b = std::pin::pin!(b);
  match futures_util::future::select(a, b).await {
    Either::Left(_) | Either::Right(_) => {}
  }
}

// ───────────────────────── TLS material ──────────────────────────

/// Optional TLS material the builder can attach to a TLS-mode server.
///
/// `Acme` / `Resolver` variants are reserved for future expansion; today only
/// PEM paths are accepted.
#[derive(Debug, Clone)]
pub enum TlsCert {
  /// Filesystem paths for cert + key PEM files.
  PemPaths {
    /// Path to the PEM-encoded certificate chain.
    cert_path: String,
    /// Path to the PEM-encoded private key.
    key_path: String,
  },
}

impl TlsCert {
  /// Construct from filesystem paths.
  pub fn pem_paths(cert: impl Into<String>, key: impl Into<String>) -> Self {
    Self::PemPaths {
      cert_path: cert.into(),
      key_path: key.into(),
    }
  }
}

// ───────────────────────── tokio Server ──────────────────────────

/// Fluent constructor for the tokio-runtime [`Server`].
#[cfg(not(feature = "compio"))]
#[derive(Debug, Default, Clone)]
pub struct ServerBuilder {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(not(feature = "compio"))]
impl ServerBuilder {
  /// Override the [`ServerConfig`] (drain timeout, h2 caps, max_connections, …).
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
#[cfg(not(feature = "compio"))]
#[derive(Debug, Clone)]
pub struct Server {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(not(feature = "compio"))]
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
  #[cfg(feature = "tls")]
  pub fn spawn_tls(&self, listener: TcpListener, router: Router) -> ServerHandle {
    let tls = self
      .tls
      .clone()
      .expect("Server::spawn_tls requires a TlsCert (use builder().tls(...))");
    let (handle, shutdown_fut) = make_handle(self.config.drain_timeout);
    let config = self.config.clone();
    spawn_done(handle.done.clone(), async move {
      let TlsCert::PemPaths { cert_path, key_path } = tls;
      crate::server_tls::serve_tls_with_shutdown_and_config(
        listener,
        router,
        Some(cert_path.as_str()),
        Some(key_path.as_str()),
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
      let TlsCert::PemPaths { cert_path, key_path } = tls;
      crate::server_h3::serve_h3_with_shutdown_and_config(
        router,
        &addr,
        Some(cert_path.as_str()),
        Some(key_path.as_str()),
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

// ───────────────────────── compio Server ──────────────────────────

/// Fluent constructor for the compio-runtime [`CompioServer`].
#[cfg(feature = "compio")]
#[derive(Debug, Default, Clone)]
pub struct CompioServerBuilder {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(feature = "compio")]
impl CompioServerBuilder {
  /// Override the [`ServerConfig`].
  #[must_use]
  pub fn config(mut self, config: ServerConfig) -> Self {
    self.config = config;
    self
  }

  /// Attach TLS material so [`CompioServer::spawn_tls`] becomes usable.
  #[must_use]
  pub fn tls(mut self, cert: TlsCert) -> Self {
    self.tls = Some(cert);
    self
  }

  /// Finalize and produce the [`CompioServer`].
  pub fn build(self) -> CompioServer {
    CompioServer {
      config: self.config,
      tls: self.tls,
    }
  }
}

/// Compio-runtime server entry point. Construct with [`CompioServer::builder`].
///
/// Mirrors the tokio [`Server`] API but drives the compio runtime — io_uring
/// on Linux, IOCP on Windows, kqueue on macOS — under the hood.
#[cfg(feature = "compio")]
#[derive(Debug, Clone)]
pub struct CompioServer {
  config: ServerConfig,
  tls: Option<TlsCert>,
}

#[cfg(feature = "compio")]
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
    spawn_done_compio(handle.done.clone(), async move {
      let TlsCert::PemPaths { cert_path, key_path } = tls;
      crate::server_tls_compio::serve_tls_with_shutdown_and_config(
        listener,
        router,
        Some(cert_path.as_str()),
        Some(key_path.as_str()),
        shutdown_fut,
        config,
      )
      .await;
    });
    handle
  }
}

// ───────────────────────── helpers ──────────────────────────

fn make_handle(drain_timeout: Duration) -> (ServerHandle, impl Future<Output = ()> + Send + 'static) {
  let shutdown = Arc::new(Notify::new());
  let done = Arc::new(Notify::new());
  let shutdown_for_task = shutdown.clone();
  // Hold the Arc inside the future so it stays alive across the spawn move,
  // and call notified() *inside* an async block so the same NotifyFuture is
  // polled across wakeups (a fresh notified() per poll loses the racing
  // notify_waiters() and deadlocks).
  let fut = async move {
    shutdown_for_task.notified().await;
  };
  (
    ServerHandle {
      shutdown,
      done,
      drain_timeout,
    },
    fut,
  )
}

#[cfg(not(feature = "compio"))]
fn spawn_done<F>(done: Arc<Notify>, fut: F)
where
  F: Future<Output = ()> + Send + 'static,
{
  tokio::spawn(async move {
    fut.await;
    done.notify_waiters();
  });
}

#[cfg(feature = "compio")]
fn spawn_done_compio<F>(done: Arc<Notify>, fut: F)
where
  F: Future<Output = ()> + 'static,
{
  compio::runtime::spawn(async move {
    fut.await;
    done.notify_waiters();
  })
  .detach();
}
