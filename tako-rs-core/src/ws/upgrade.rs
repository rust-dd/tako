use std::future::Future;
use std::marker::PhantomData;
use std::time::Duration;

use futures_util::future::Either;
use http::HeaderValue;
use http::StatusCode;
use http::header;
use hyper::upgrade::OnUpgrade;

use super::WebSocket;
use super::driver::Driver;
use super::handshake;
use super::runtime;
use super::socket::Config;
use super::socket::Core;
use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

#[cfg(test)]
mod tests;

/// Answers an HTTP/1.1 upgrade request and runs `handler` on the WebSocket connection.
/// Invalid buffer sizes or limits that exceed addressable allocation sizes return HTTP 500.
pub struct WebSocketUpgrade<H, Fut> {
  request: Request,
  handler: H,
  protocols: Vec<&'static str>,
  allowed_origins: Option<Vec<String>>,
  config: Config,
  upgrade_timeout: Option<Duration>,
  max_lifetime: Option<Duration>,
  _future: PhantomData<fn() -> Fut>,
}

impl<H, Fut> WebSocketUpgrade<H, Fut>
where
  H: FnOnce(WebSocket) -> Fut,
  Fut: Future<Output = ()>,
{
  pub fn new(request: Request, handler: H) -> Self {
    Self {
      request,
      handler,
      protocols: Vec::new(),
      allowed_origins: None,
      config: Config::default(),
      upgrade_timeout: None,
      max_lifetime: None,
      _future: PhantomData,
    }
  }

  /// Subprotocols the server speaks, in order of preference.
  pub fn protocols<I, S>(mut self, list: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<&'static str>,
  {
    self.protocols = list.into_iter().map(Into::into).collect();
    self
  }

  /// Accepts only these `Origin` values; without the list every origin passes.
  pub fn allowed_origins<I, S>(mut self, origins: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.allowed_origins = Some(origins.into_iter().map(Into::into).collect());
    self
  }

  /// Largest frame payload accepted, 16 MiB by default.
  pub fn max_frame_size(mut self, n: usize) -> Self {
    self.config.limits.max_frame_size = n;
    self
  }

  /// Largest message accepted, 64 MiB by default.
  pub fn max_message_size(mut self, n: usize) -> Self {
    self.config.limits.max_message_size = n;
    self
  }

  /// Initial read buffer capacity, 4 KiB by default; the buffer grows to fit larger frames.
  /// Must be between 1 byte and 64 KiB.
  pub fn read_buffer_size(mut self, n: usize) -> Self {
    self.config.read_buffer_size = n;
    self
  }

  /// Queued bytes past which `send` writes at once, 64 KiB by default.
  pub fn write_buffer_size(mut self, n: usize) -> Self {
    self.config.write_buffer_size = n;
    self
  }

  /// How long to wait for the server to hand over the upgraded connection.
  pub fn upgrade_timeout(mut self, d: Duration) -> Self {
    self.upgrade_timeout = Some(d);
    self
  }

  /// Closes the connection with 1001 once it has been open this long.
  pub fn max_lifetime(mut self, d: Duration) -> Self {
    self.max_lifetime = Some(d);
    self
  }

  fn accept(self) -> (Response, Option<Session<H>>) {
    if !self.config.is_valid() {
      tracing::error!(config = ?self.config, "invalid WebSocket buffer configuration");
      return (
        handshake::rejection(StatusCode::INTERNAL_SERVER_ERROR),
        None,
      );
    }
    let accept = match handshake::accept(&self.request) {
      Ok(accept) => accept,
      Err(status) => return (handshake::rejection(status), None),
    };
    if !handshake::origin_allowed(self.request.headers(), self.allowed_origins.as_deref()) {
      let forbidden = http::Response::builder()
        .status(StatusCode::FORBIDDEN)
        .body(TakoBody::from("origin not allowed"))
        .expect("valid forbidden response");
      return (forbidden, None);
    }
    let mut response = http::Response::builder()
      .status(StatusCode::SWITCHING_PROTOCOLS)
      .header(header::UPGRADE, "websocket")
      .header(header::CONNECTION, "Upgrade")
      .header(header::SEC_WEBSOCKET_ACCEPT, accept);
    if let Some(protocol) =
      handshake::negotiate_subprotocol(self.request.headers(), &self.protocols)
    {
      response = response.header(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(protocol),
      );
    }
    let response = response
      .body(TakoBody::empty())
      .expect("valid upgrade response");
    let Self {
      mut request,
      handler,
      config,
      upgrade_timeout,
      max_lifetime,
      ..
    } = self;
    let session = request
      .extensions_mut()
      .remove::<OnUpgrade>()
      .map(|on_upgrade| Session {
        on_upgrade,
        handler,
        config,
        upgrade_timeout,
        max_lifetime,
      });
    (response, session)
  }
}

struct Session<H> {
  on_upgrade: OnUpgrade,
  handler: H,
  config: Config,
  upgrade_timeout: Option<Duration>,
  max_lifetime: Option<Duration>,
}

impl<H> Session<H> {
  async fn run<Fut>(self)
  where
    H: FnOnce(WebSocket) -> Fut,
    Fut: Future<Output = ()>,
  {
    let upgraded = match self.upgrade_timeout {
      Some(limit) => {
        match futures_util::future::select(self.on_upgrade, runtime::sleep(limit)).await {
          Either::Left((Ok(upgraded), _)) => upgraded,
          _ => return,
        }
      }
      None => match self.on_upgrade.await {
        Ok(upgraded) => upgraded,
        Err(_) => return,
      },
    };
    let core = runtime::shared(Core::new(upgraded, self.config));
    let socket = WebSocket::new(core.clone());
    Driver::new(core, (self.handler)(socket), self.max_lifetime).await;
  }
}

/// Runs `handler` on the connection once the server hands it over, with default settings.
#[cfg(all(feature = "async-graphql", not(feature = "compio")))]
pub(crate) fn spawn_session<H, Fut>(on_upgrade: OnUpgrade, handler: H)
where
  H: FnOnce(WebSocket) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  let session = Session {
    on_upgrade,
    handler,
    config: Config::default(),
    upgrade_timeout: None,
    max_lifetime: None,
  };
  runtime::spawn(session.run());
}

#[cfg(not(feature = "compio"))]
impl<H, Fut> Responder for WebSocketUpgrade<H, Fut>
where
  H: FnOnce(WebSocket) -> Fut + Send + 'static,
  Fut: Future<Output = ()> + Send + 'static,
{
  fn into_response(self) -> Response {
    let (response, session) = self.accept();
    if let Some(session) = session {
      runtime::spawn(session.run());
    }
    response
  }
}

#[cfg(feature = "compio")]
impl<H, Fut> Responder for WebSocketUpgrade<H, Fut>
where
  H: FnOnce(WebSocket) -> Fut + 'static,
  Fut: Future<Output = ()> + 'static,
{
  fn into_response(self) -> Response {
    let (response, session) = self.accept();
    if let Some(session) = session {
      runtime::spawn(session.run());
    }
    response
  }
}
