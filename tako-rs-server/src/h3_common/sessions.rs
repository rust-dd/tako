//! Hand-off of accepted WebTransport sessions from request tasks to the
//! connection loop, which owns the h3 connection a session takes over.
//!
//! Without the `webtransport` feature the hand-off never fires and costs
//! nothing: the loop's `recv` arm stays pending forever.

use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(not(feature = "webtransport"))]
use std::time::Duration;

use bytes::Bytes;
use h3::server::RequestStream;
use http::Request;
use tako_rs_core::router::Router;
use tako_rs_core::types::BoxError;
#[cfg(not(feature = "webtransport"))]
use tokio_util::sync::CancellationToken;

use super::connection::QuicConnection;
use super::request::handle_request;

/// The request stream of one HTTP/3 request on this runtime's QUIC backend.
pub(crate) type ServerStream =
  RequestStream<<QuicConnection as h3::quic::OpenStreams<Bytes>>::BidiStream, Bytes>;

#[cfg(feature = "webtransport")]
pub(crate) use crate::webtransport::upgrade::Accepted;

/// No session can be accepted without the `webtransport` feature.
#[cfg(not(feature = "webtransport"))]
pub(crate) enum Accepted {}

#[cfg(not(feature = "webtransport"))]
impl Accepted {
  pub(crate) fn run(
    self,
    _conn: h3::server::Connection<QuicConnection, Bytes>,
    _router: Arc<Router>,
    _remote_addr: SocketAddr,
    _shutdown: &CancellationToken,
    _grace: Duration,
  ) -> std::future::Ready<()> {
    match self {}
  }
}

pub(crate) struct Sessions {
  #[cfg(feature = "webtransport")]
  tx: tokio::sync::mpsc::UnboundedSender<Accepted>,
  #[cfg(feature = "webtransport")]
  rx: tokio::sync::mpsc::UnboundedReceiver<Accepted>,
}

impl Sessions {
  pub(crate) fn new() -> Self {
    #[cfg(feature = "webtransport")]
    {
      let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
      Self { tx, rx }
    }
    #[cfg(not(feature = "webtransport"))]
    Self {}
  }

  #[cfg_attr(not(feature = "webtransport"), allow(clippy::unused_self))]
  pub(crate) fn sender(&self) -> SessionSender {
    SessionSender {
      #[cfg(feature = "webtransport")]
      tx: self.tx.clone(),
    }
  }

  /// Resolves once a request task hands over an accepted session.
  pub(crate) async fn recv(&mut self) -> Accepted {
    #[cfg(feature = "webtransport")]
    if let Some(accepted) = self.rx.recv().await {
      return accepted;
    }
    std::future::pending().await
  }
}

#[derive(Clone)]
pub(crate) struct SessionSender {
  #[cfg(feature = "webtransport")]
  tx: tokio::sync::mpsc::UnboundedSender<Accepted>,
}

impl SessionSender {
  /// Serves one HTTP/3 request. A WebTransport CONNECT goes through the router
  /// first, and an accepted session is handed to the connection loop.
  pub(crate) async fn serve(
    self,
    req: Request<()>,
    stream: ServerStream,
    router: Arc<Router>,
    remote_addr: SocketAddr,
  ) -> Result<(), BoxError> {
    #[cfg(feature = "webtransport")]
    if crate::webtransport::upgrade::is_session_request(&req) {
      if let Some(accepted) =
        crate::webtransport::upgrade::route(req, stream, &router, remote_addr).await
      {
        // The loop stops listening once it runs a session; later CONNECTs
        // exceed the one-session limit and are dropped.
        let _ = self.tx.send(accepted);
      }
      return Ok(());
    }
    handle_request(req, stream, router, remote_addr).await
  }
}
