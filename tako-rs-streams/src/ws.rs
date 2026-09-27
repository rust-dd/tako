//! WebSocket connection handling and message processing utilities.
//!
//! `TakoWs<H>` performs the RFC-6455 server-side handshake and hands the
//! upgraded stream to a handler. The builder configures subprotocols, size limits,
//! allowed origins, upgrade timeout, and total connection lifetime.
//!
//! Handlers own the stream and must implement application ping/pong loops themselves.

use std::future::Future;
use std::time::Duration;

use futures_util::FutureExt;
use http::HeaderValue;
use http::StatusCode;
use http::header;
use hyper::upgrade::Upgraded;
use hyper_util::rt::TokioIo;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;

/// Legacy configuration accepted by the deprecated, ineffective `keep_alive` method.
/// Applications should capture their timing configuration in the handler closure.
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
  /// Hard cap on how long a single WebSocket conversation may live after a
  /// successful upgrade. When set, the handler future is wrapped in
  /// `tokio::time::timeout(max_lifetime, …)`; expiry drops the connection.
  /// Defends against slowloris-style holders that never send data after
  /// upgrade.
  max_lifetime: Option<Duration>,
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
      max_lifetime: None,
    }
  }

  /// Hard-cap on total connection lifetime after upgrade. See
  /// [`Self::max_lifetime`]-field docs.
  pub fn max_lifetime(mut self, d: Duration) -> Self {
    self.max_lifetime = Some(d);
    self
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

  /// This legacy method has no effect. Implement ping/pong timing inside the handler.
  #[deprecated(note = "no effect; implement ping/pong in the handler or use max_lifetime")]
  pub fn keep_alive(self, _config: WsKeepAlive) -> Self {
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
    let offered: Vec<&str> = header.split(',').map(str::trim).collect();
    self
      .protocols
      .iter()
      .copied()
      .find(|server_pref| offered.contains(server_pref))
  }

  fn origin_allowed(&self, headers: &http::HeaderMap) -> bool {
    let Some(allowed) = self.allowed_origins.as_ref() else {
      return true;
    };
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else {
      return false;
    };
    let observed = normalize_origin(origin);
    allowed
      .iter()
      .any(|a| normalize_origin(a) == observed && !observed.is_empty())
  }
}

/// Normalize scheme, host, and port for the origin allow-list.
fn normalize_origin(raw: &str) -> String {
  let raw = raw.trim();
  if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
    return String::new();
  }
  let Ok(url) = url::Url::parse(raw) else {
    return String::new();
  };
  // Reject userinfo — an Origin must not carry credentials.
  if !url.username().is_empty() || url.password().is_some() {
    return String::new();
  }
  let scheme = url.scheme().to_ascii_lowercase();
  let Some(host) = url.host_str() else {
    return String::new();
  };
  let host = host.to_ascii_lowercase();
  let port = url.port();
  let default = matches!(
    (scheme.as_str(), port),
    ("http" | "ws", Some(80)) | ("https" | "wss", Some(443))
  );
  match port {
    Some(p) if !default => format!("{scheme}://{host}:{p}"),
    _ => format!("{scheme}://{host}"),
  }
}

impl<H, Fut> Responder for TakoWs<H, Fut>
where
  H: FnOnce(WebSocketStream<TokioIo<Upgraded>>) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  fn into_response(self) -> Response {
    let accept = match crate::ws_handshake::accept(&self.request) {
      Ok(accept) => accept,
      Err(status) => return crate::ws_handshake::rejection(status),
    };
    let ws_config = self.websocket_config();
    if !self.origin_allowed(self.request.headers()) {
      return http::Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(TakoBody::from("origin not allowed"))
        .expect("valid forbidden response");
    }
    let selected_proto = self.negotiate_subprotocol(self.request.headers());
    let upgrade_timeout = self.upgrade_timeout;
    let max_lifetime = self.max_lifetime;

    let TakoWs {
      request, handler, ..
    } = self;
    let (parts, body) = request.into_parts();
    let req = http::Request::from_parts(parts, body);

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
        let handler_fut =
          std::panic::AssertUnwindSafe(async move { handler(ws).await }).catch_unwind();
        match max_lifetime {
          Some(d) => {
            let _ = tokio::time::timeout(d, handler_fut).await;
          }
          None => {
            let _ = handler_fut.await;
          }
        }
      });
    }

    response
  }
}

#[cfg(test)]
mod tests {
  use super::normalize_origin;

  #[test]
  fn normalize_origin_lowercases_scheme_and_host() {
    assert_eq!(
      normalize_origin("HTTPS://Example.COM"),
      "https://example.com"
    );
  }

  #[test]
  fn normalize_origin_strips_default_ports() {
    assert_eq!(
      normalize_origin("http://example.com:80"),
      "http://example.com"
    );
    assert_eq!(
      normalize_origin("https://example.com:443"),
      "https://example.com"
    );
  }

  #[test]
  fn normalize_origin_keeps_nondefault_ports() {
    assert_eq!(
      normalize_origin("https://example.com:8443"),
      "https://example.com:8443"
    );
  }

  #[test]
  fn normalize_origin_rejects_malformed_or_null() {
    assert_eq!(normalize_origin(""), "");
    assert_eq!(normalize_origin("null"), "");
    assert_eq!(normalize_origin("not-an-origin"), "");
  }

  #[test]
  fn normalize_origin_ignores_trailing_path() {
    assert_eq!(
      normalize_origin("https://example.com/path?x=1"),
      "https://example.com"
    );
  }
}
