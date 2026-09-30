//! HTTP server entry points for the compio runtime.
//!
//! Every listener (TCP HTTP/1, h2c, PROXY protocol, Unix sockets) runs the
//! same accept loop
//! in `accept` and serves connections through `connection`.

use std::future::Future;

use compio::net::TcpListener;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;

use crate::ServerConfig;

mod accept;
mod connection;
#[cfg(unix)]
pub(crate) mod unix;

use accept::accept_loop;
use connection::Protocol;

#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve(listener: TcpListener, router: Router) {
  if let Err(e) = run(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("Server error: {e}");
  }
}

/// Starts the Tako HTTP server (compio) with graceful shutdown support.
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_shutdown(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_config(listener: TcpListener, router: Router, config: ServerConfig) {
  if let Err(e) = run(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("Server error: {e}");
  }
}

/// Like [`serve_with_shutdown`] with caller-supplied [`ServerConfig`].
#[deprecated(
  since = "2.1.0",
  note = "use Server::builder() or CompioServer::builder() and a try_spawn_* method"
)]
pub async fn serve_with_shutdown_and_config(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run(listener, router, Some(signal), config).await {
    tracing::error!("Server error: {e}");
  }
}

pub(crate) async fn run(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::Http1).await
}

/// HTTP/2 prior knowledge (h2c) over cleartext TCP.
#[cfg(feature = "http2")]
pub(crate) async fn run_h2c(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::H2c).await
}

/// HTTP/1 behind a PROXY protocol v1/v2 header.
#[cfg(feature = "proxy-protocol")]
pub(crate) async fn run_proxy_protocol(
  listener: TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::ProxyHttp1).await
}

/// HTTP/1 over a Unix domain socket.
#[cfg(unix)]
pub(crate) async fn run_unix(
  listener: unix::UnixSocketListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::Http1).await
}
