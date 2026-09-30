//! W3C WebTransport over HTTP/3, on the Tokio and compio servers.
//!
//! A browser's `new WebTransport("https://example.com/echo")` arrives at the
//! HTTP/3 server as an extended CONNECT request. It runs through the router
//! like any other request, so routing and middleware such as authentication
//! apply. The route's handler takes the
//! [`WebTransport`](crate::webtransport::WebTransport) extractor and returns
//! [`WebTransport::on_session`](crate::webtransport::WebTransport::on_session);
//! the callback receives the
//! [`WebTransportSession`](crate::webtransport::WebTransportSession) once the
//! server has accepted it.
//!
//! ```rust,no_run
//! # #[cfg(not(feature = "compio"))]
//! # mod example {
//! use tako::Method;
//! use tako::router::Router;
//! use tako::webtransport::{WebTransport, WebTransportSession};
//!
//! async fn echo(wt: WebTransport) -> tako::types::Response {
//!   wt.on_session(|session: WebTransportSession| async move {
//!     while let Ok(Some(stream)) = session.accept_bi().await {
//!       tokio::spawn(async move {
//!         let (mut recv, mut send) = tokio::io::split(stream);
//!         let _ = tokio::io::copy(&mut recv, &mut send).await;
//!       });
//!     }
//!   })
//! }
//!
//! fn router() -> Router {
//!   let mut router = Router::new();
//!   router.route(Method::CONNECT, "/echo", echo);
//!   router
//! }
//! # }
//! ```
//!
//! Serve the router with `try_spawn_h3`. Each QUIC connection carries at most
//! one session; other HTTP/3 requests on that connection still reach the
//! router.

use std::future::Future;

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

mod capsule;
mod driver;
mod session;
mod stream;
pub(crate) mod upgrade;

pub use capsule::WebTransportClose;
pub use h3::webtransport::SessionId;
pub use session::WebTransportSession;
pub use stream::BidiStream;
pub use stream::RecvStream;
pub use stream::SendStream;
use upgrade::SessionSlot;

/// Extractor for a WebTransport session request.
///
/// Available only on an HTTP/3 extended CONNECT request with
/// `:protocol = webtransport`; anything else is rejected with
/// [`WebTransportRejection`].
pub struct WebTransport {
  slot: SessionSlot,
}

impl WebTransport {
  /// Accepts the session and returns the handler's `200 OK`.
  ///
  /// `handler` runs once the server has sent the response. Returning a
  /// non-2xx response instead of calling this rejects the session.
  #[cfg(not(feature = "compio"))]
  pub fn on_session<F, Fut>(self, handler: F) -> Response
  where
    F: FnOnce(WebTransportSession) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + Send + 'static,
  {
    self
      .slot
      .fill(Box::new(move |session| Box::pin(handler(session))));
    Response::new(TakoBody::empty())
  }

  /// Accepts the session and returns the handler's `200 OK`.
  ///
  /// `handler` runs on the connection's runtime thread once the server has
  /// sent the response, so its future need not be `Send`. Returning a
  /// non-2xx response instead of calling this rejects the session.
  #[cfg(feature = "compio")]
  pub fn on_session<F, Fut>(self, handler: F) -> Response
  where
    F: FnOnce(WebTransportSession) -> Fut + Send + 'static,
    Fut: Future<Output = ()> + 'static,
  {
    self
      .slot
      .fill(Box::new(move |session| Box::pin(handler(session))));
    Response::new(TakoBody::empty())
  }
}

impl<'a> FromRequestParts<'a> for WebTransport {
  type Error = WebTransportRejection;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(take(&mut parts.extensions))
  }
}

impl<'a> FromRequest<'a> for WebTransport {
  type Error = WebTransportRejection;

  fn from_request(
    req: &'a mut Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(take(req.extensions_mut()))
  }
}

fn take(extensions: &mut http::Extensions) -> Result<WebTransport, WebTransportRejection> {
  extensions
    .remove::<SessionSlot>()
    .map(|slot| WebTransport { slot })
    .ok_or(WebTransportRejection)
}

/// Rejection for a request that is not a WebTransport session request.
///
/// Responds with `400 Bad Request`.
#[derive(Debug)]
pub struct WebTransportRejection;

impl Responder for WebTransportRejection {
  fn into_response(self) -> Response {
    (
      StatusCode::BAD_REQUEST,
      "expected a WebTransport session request",
    )
      .into_response()
  }
}
