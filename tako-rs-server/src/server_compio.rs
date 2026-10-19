//! Compio HTTP listeners (TCP HTTP/1, h2c, PROXY protocol, Unix sockets) sharing one accept loop.

use std::future::Future;

use compio::net::TcpListener;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;

use crate::ServerConfig;

mod accept;
mod connection;
#[cfg(feature = "http3")]
pub(crate) mod h3;
#[cfg(unix)]
pub(crate) mod unix;

use accept::accept_loop;
use connection::Protocol;

pub(crate) async fn run(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::Http1).await
}

#[cfg(feature = "http2")]
pub(crate) async fn run_h2c(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::H2c).await
}

#[cfg(feature = "proxy-protocol")]
pub(crate) async fn run_proxy_protocol(
  listener: TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::ProxyHttp1).await
}

#[cfg(unix)]
pub(crate) async fn run_unix(
  listener: unix::UnixSocketListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  accept_loop(listener, router, signal, config, Protocol::Http1).await
}
