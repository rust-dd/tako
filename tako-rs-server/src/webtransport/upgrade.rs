//! Routing of WebTransport CONNECT requests and the hand-over of the h3
//! connection to an accepted session.

use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::time::Duration;

use bytes::Bytes;
use h3::ext::Protocol;
use http::Method;
use http::Request;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tokio_util::sync::CancellationToken;

use super::WebTransportSession;
use crate::h3_common::connection::QuicConnection;
use crate::h3_common::request::annotate;
use crate::h3_common::request::respond;
use crate::h3_common::sessions::ServerStream;

#[cfg(not(feature = "compio"))]
type SessionFuture = Pin<Box<dyn Future<Output = ()> + Send>>;
#[cfg(feature = "compio")]
type SessionFuture = Pin<Box<dyn Future<Output = ()>>>;

type SessionHandler = Box<dyn FnOnce(WebTransportSession) -> SessionFuture + Send>;

/// Where `WebTransport::on_session` leaves its callback. The server finds it
/// after the router has answered the CONNECT request.
#[derive(Clone, Default)]
pub(crate) struct SessionSlot(Arc<Mutex<Option<SessionHandler>>>);

impl SessionSlot {
  pub(super) fn fill(&self, handler: SessionHandler) {
    *self.0.lock().unwrap_or_else(PoisonError::into_inner) = Some(handler);
  }

  fn take(&self) -> Option<SessionHandler> {
    self.0.lock().unwrap_or_else(PoisonError::into_inner).take()
  }
}

/// A CONNECT request the router accepted, waiting for the h3 connection.
pub(crate) struct Accepted {
  stream: ServerStream,
  handler: SessionHandler,
}

pub(crate) fn is_session_request(req: &Request<()>) -> bool {
  req.method() == Method::CONNECT
    && req.extensions().get::<Protocol>() == Some(&Protocol::WEB_TRANSPORT)
}

/// Runs a WebTransport CONNECT request through the router.
///
/// Returns the request when a handler called `on_session` and the response is
/// 2xx. Any other response goes back to the client as is.
pub(crate) async fn route(
  request: Request<()>,
  mut stream: ServerStream,
  router: &Router,
  remote_addr: SocketAddr,
) -> Option<Accepted> {
  let slot = SessionSlot::default();
  let mut routed = Request::new(TakoBody::empty());
  *routed.method_mut() = request.method().clone();
  *routed.uri_mut() = request.uri().clone();
  *routed.version_mut() = request.version();
  *routed.headers_mut() = request.headers().clone();
  *routed.extensions_mut() = request.extensions().clone();
  annotate(&mut routed, remote_addr);
  routed.extensions_mut().insert(slot.clone());

  let response = router.dispatch(routed).await;
  let handler = slot.take();
  if response.status().is_success() {
    if let Some(handler) = handler {
      return Some(Accepted { stream, handler });
    }
    tracing::warn!("WebTransport route answered 2xx without calling WebTransport::on_session");
  }
  if let Err(e) = respond(&mut stream, response).await {
    tracing::debug!("WebTransport rejection error: {e}");
  }
  None
}

impl Accepted {
  /// Accepts the session on `conn` and runs the handler until it returns or
  /// `grace` has passed since the session or the server began to close.
  pub(crate) async fn run(
    self,
    conn: h3::server::Connection<QuicConnection, Bytes>,
    router: Arc<Router>,
    remote_addr: SocketAddr,
    shutdown: &CancellationToken,
    grace: Duration,
  ) {
    WebTransportSession::run(
      self.stream,
      conn,
      self.handler,
      router,
      remote_addr,
      shutdown,
      grace,
    )
    .await;
  }
}
