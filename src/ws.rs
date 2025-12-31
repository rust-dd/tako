//! WebSocket connection handling and message processing utilities.
//!
//! This module provides the `TakoWs` struct for handling WebSocket upgrade requests and
//! processing WebSocket connections. It implements the WebSocket handshake protocol
//! according to RFC 6455, manages connection upgrades, and provides a clean interface
//! for handling WebSocket streams. The module integrates with Tako's responder system
//! to enable seamless WebSocket support in web applications.
//!
//! # Examples
//!
//! ```rust
//! use tako::ws::TakoWs;
//! use tako::types::Request;
//! use tako::body::TakoBody;
//! use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
//! use hyper_util::rt::TokioIo;
//! use futures_util::{StreamExt, SinkExt};
//!
//! async fn websocket_handler(mut ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>) {
//!     while let Some(msg) = ws.next().await {
//!         match msg {
//!             Ok(Message::Text(text)) => {
//!                 println!("Received: {}", text);
//!                 let _ = ws.send(Message::Text(format!("Echo: {}", text))).await;
//!             }
//!             Ok(Message::Close(_)) => break,
//!             _ => {}
//!         }
//!     }
//! }
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! let request = Request::builder()
//!     .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
//!     .header("upgrade", "websocket")
//!     .header("connection", "upgrade")
//!     .body(TakoBody::empty())?;
//!
//! let ws = TakoWs::new(request, websocket_handler);
//! # Ok(())
//! # }
//! ```

use std::future::Future;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::FutureExt;
use http::StatusCode;
use http::header;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use sha1::Digest;
use sha1::Sha1;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// WebSocket connection handler with upgrade protocol support.
///
/// `TakoWs` manages the WebSocket handshake process and connection upgrade from HTTP
/// to WebSocket protocol. It validates the WebSocket upgrade request, performs the
/// RFC 6455 handshake, and spawns a task to handle the WebSocket connection using
/// the provided handler function.
///
/// # Type Parameters
///
/// * `H` - Handler function type that processes the WebSocket connection
/// * `Fut` - Future type returned by the handler function
///
/// # Examples
///
/// ```rust
/// use tako::ws::TakoWs;
/// use tako::types::Request;
/// use tako::body::TakoBody;
/// use tokio_tungstenite::{WebSocketStream, tungstenite::Message};
/// use hyper_util::rt::TokioIo;
/// use futures_util::{StreamExt, SinkExt};
///
/// async fn echo_handler(mut ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>>) {
///     while let Some(msg) = ws.next().await {
///         if let Ok(Message::Text(text)) = msg {
///             let _ = ws.send(Message::Text(format!("Echo: {}", text))).await;
///         }
///     }
/// }
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// let request = Request::builder()
///     .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
///     .header("upgrade", "websocket")
///     .header("connection", "upgrade")
///     .body(TakoBody::empty())?;
///
/// let ws_handler = TakoWs::new(request, echo_handler);
/// # Ok(())
/// # }
/// ```
#[doc(alias = "websocket")]
#[doc(alias = "ws")]
pub struct TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  request: Request,
  handler: H,
}

impl<H, Fut> TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  /// Creates a new WebSocket handler with the given request and handler function.
  pub fn new(request: Request, handler: H) -> Self {
    Self { request, handler }
  }
}

impl<H, Fut> Responder for TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  /// Converts the WebSocket handler into an HTTP response with upgrade protocol.
  fn into_response(self) -> Response {
    let (parts, body) = self.request.into_parts();
    let req = http::Request::from_parts(parts, body);

    let key = match req.headers().get("Sec-WebSocket-Key") {
      Some(k) => k,
      None => {
        return http::Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .body(TakoBody::from("Missing Sec-WebSocket-Key".to_string()))
          .unwrap();
      }
    };

    // RFC‑6455 accept hash
    let accept = {
      let mut sha1 = Sha1::new();
      sha1.update(key.as_bytes());
      sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
      STANDARD.encode(sha1.finalize())
    };

    let response = http::Response::builder()
      .status(StatusCode::SWITCHING_PROTOCOLS)
      .header(header::UPGRADE, "websocket")
      .header(header::CONNECTION, "Upgrade")
      .header("Sec-WebSocket-Accept", accept)
      .body(TakoBody::empty())
      .unwrap();

    if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
      let handler = self.handler;
      tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
          let upgraded = TokioIo::new(upgraded);
          let ws = WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await;
          let _ = std::panic::AssertUnwindSafe(handler(ws))
            .catch_unwind()
            .await;
        }
      });
    }

    response
  }
}
