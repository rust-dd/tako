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

use crate::{
  body::TakoBody,
  responder::Responder,
  types::{Request, Response},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use compio::{
  net::TcpStream,
  ws::{WebSocketStream, accept_async},
};
use futures_util::FutureExt;
use http::{StatusCode, header};
use sha1::{Digest, Sha1};
use std::future::Future;

#[doc(alias = "websocket_compio")]
#[doc(alias = "ws_compio")]
pub struct TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  request: Request,
  handler: H,
  stream: TcpStream,
}

impl<H, Fut> TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  /// Creates a new WebSocket handler with the given request and handler function.
  pub fn new(request: Request, handler: H, stream: TcpStream) -> Self {
    Self {
      request,
      handler,
      stream,
    }
  }
}

impl<H, Fut> Responder for TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TcpStream>) -> Fut + Send + 'static,
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
      compio::runtime::spawn(async move {
        let ws = accept_async(self.stream).await;

        if let Err(e) = ws {
          tracing::error!("WebSocket accept error: {e}");
          return;
        }

        let ws = ws.unwrap();
        if let Ok(_) = on_upgrade.await {
          let _ = std::panic::AssertUnwindSafe(handler(ws))
            .catch_unwind()
            .await;
        }
      })
      .detach();
    }

    response
  }
}
