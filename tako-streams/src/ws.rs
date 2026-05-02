//! WebSocket connection handling and message processing utilities.
//!
//! `TakoWs<H>` performs the RFC-6455 server-side handshake and hands the
//! upgraded stream to a user-supplied handler. v2 builder additions:
//!
//! - subprotocol negotiation (echoes the first match from a configured list)
//! - per-connection size caps (`max_frame_size`, `max_message_size`)
//! - origin allow-list (rejects mismatching `Origin` with `403`)
//! - upgrade timeout (drops leaked tasks when the client never finishes the upgrade)
//! - configurable initial `WebSocketConfig` (forwarded to tokio-tungstenite)
//!
//! Application-level keep-alive (`ping_interval` / `pong_timeout`) is exposed
//! as a [`WsKeepAlive`] config value the handler can read; the framework
//! itself does not run the ping loop because the handler owns the stream.

use std::future::Future;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use futures_util::FutureExt;
use http::HeaderValue;
use http::StatusCode;
use http::header;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use sha1::Digest;
use sha1::Sha1;
use tako_core::body::TakoBody;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// Application-level keep-alive hints attached to the `TakoWs` builder.
#[derive(Debug, Clone, Copy, Default)]
pub struct WsKeepAlive {
  /// Period between server-initiated pings; `None` disables.
  pub ping_interval: Option<Duration>,
  /// Maximum time to wait for a pong reply before treating the connection as dead.
  pub pong_timeout: Option<Duration>,
}

/// WebSocket connection handler with upgrade protocol support.
#[doc(alias = "websocket")]
#[doc(alias = "ws")]
pub struct TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  request: Request,
  handler: H,
  protocols: Vec<&'static str>,
  max_frame_size: Option<usize>,
  max_message_size: Option<usize>,
  allowed_origins: Option<Vec<String>>,
  upgrade_timeout: Option<Duration>,
  keep_alive: WsKeepAlive,
}

impl<H, Fut> TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  /// Creates a new WebSocket handler with the given request and handler function.
  pub fn new(request: Request, handler: H) -> Self {
    Self {
      request,
      handler,
      protocols: Vec::new(),
      max_frame_size: None,
      max_message_size: None,
      allowed_origins: None,
      upgrade_timeout: None,
      keep_alive: WsKeepAlive::default(),
    }
  }

  /// Configure accepted subprotocols.
  pub fn protocols<I, S>(mut self, list: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<&'static str>,
  {
    self.protocols = list.into_iter().map(Into::into).collect();
    self
  }

  /// Limit the maximum WebSocket frame size in bytes.
  pub fn max_frame_size(mut self, n: usize) -> Self {
    self.max_frame_size = Some(n);
    self
  }

  /// Limit the maximum WebSocket message size in bytes.
  pub fn max_message_size(mut self, n: usize) -> Self {
    self.max_message_size = Some(n);
    self
  }

  /// Restrict the upgrade to clients whose `Origin` header matches the allow-list.
  pub fn allowed_origins<I, S>(mut self, origins: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.allowed_origins = Some(origins.into_iter().map(Into::into).collect());
    self
  }

  /// Cap how long the framework waits for `hyper::upgrade::OnUpgrade` to resolve.
  pub fn upgrade_timeout(mut self, d: Duration) -> Self {
    self.upgrade_timeout = Some(d);
    self
  }

  /// Configure server-initiated keep-alive hints.
  pub fn keep_alive(mut self, k: WsKeepAlive) -> Self {
    self.keep_alive = k;
    self
  }

  fn websocket_config(&self) -> Option<WebSocketConfig> {
    if self.max_frame_size.is_none() && self.max_message_size.is_none() {
      return None;
    }
    let mut cfg = WebSocketConfig::default();
    if let Some(n) = self.max_frame_size {
      cfg.max_frame_size = Some(n);
    }
    if let Some(n) = self.max_message_size {
      cfg.max_message_size = Some(n);
    }
    Some(cfg)
  }

  fn negotiate_subprotocol(&self, headers: &http::HeaderMap) -> Option<&'static str> {
    if self.protocols.is_empty() {
      return None;
    }
    let header = headers
      .get(header::SEC_WEBSOCKET_PROTOCOL)
      .and_then(|v| v.to_str().ok())?;
    for offered in header.split(',').map(str::trim) {
      if let Some(matched) = self.protocols.iter().copied().find(|p| *p == offered) {
        return Some(matched);
      }
    }
    None
  }

  fn origin_allowed(&self, headers: &http::HeaderMap) -> bool {
    let Some(allowed) = self.allowed_origins.as_ref() else {
      return true;
    };
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
      return false;
    };
    allowed.iter().any(|a| a == origin)
  }
}

impl<H, Fut> Responder for TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  fn into_response(self) -> Response {
    let ws_config = self.websocket_config();
    if !self.origin_allowed(self.request.headers()) {
      return http::Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(TakoBody::from("origin not allowed"))
        .expect("valid forbidden response");
    }
    let selected_proto = self.negotiate_subprotocol(self.request.headers());
    let upgrade_timeout = self.upgrade_timeout;

    let TakoWs {
      request, handler, ..
    } = self;
    let (parts, body) = request.into_parts();
    let req = http::Request::from_parts(parts, body);

    let key = match req.headers().get("Sec-WebSocket-Key") {
      Some(k) => k,
      None => {
        return http::Response::builder()
          .status(StatusCode::BAD_REQUEST)
          .body(TakoBody::from("Missing Sec-WebSocket-Key".to_string()))
          .expect("valid bad request response");
      }
    };

    let accept = {
      let mut sha1 = Sha1::new();
      sha1.update(key.as_bytes());
      sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
      STANDARD.encode(sha1.finalize())
    };

    let mut builder = http::Response::builder()
      .status(StatusCode::SWITCHING_PROTOCOLS)
      .header(header::UPGRADE, "websocket")
      .header(header::CONNECTION, "Upgrade")
      .header("Sec-WebSocket-Accept", accept);
    if let Some(p) = selected_proto {
      builder = builder.header(header::SEC_WEBSOCKET_PROTOCOL, HeaderValue::from_static(p));
    }

    let response = builder
      .body(TakoBody::empty())
      .expect("valid WebSocket upgrade response");

    if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
      tokio::spawn(async move {
        let upgraded = match upgrade_timeout {
          Some(d) => match tokio::time::timeout(d, on_upgrade).await {
            Ok(Ok(u)) => u,
            _ => return,
          },
          None => match on_upgrade.await {
            Ok(u) => u,
            Err(_) => return,
          },
        };
        let upgraded = TokioIo::new(upgraded);
        let ws = WebSocketStream::from_raw_socket(upgraded, Role::Server, ws_config).await;
        let _ = std::panic::AssertUnwindSafe(handler(ws))
          .catch_unwind()
          .await;
      });
    }

    response
  }
}
