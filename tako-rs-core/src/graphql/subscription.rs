//! `GraphQL`-over-WebSocket subscription responder that performs the HTTP
//! upgrade handshake and drives async-graphql's WebSocket state machine.

use std::future::Future;
use std::str::FromStr;
use std::time::Duration;

use async_graphql::Data;
use async_graphql::Executor;
use async_graphql::Result as GqlResult;
use async_graphql::http::DefaultOnConnInitType;
use async_graphql::http::DefaultOnPingType;
use async_graphql::http::WebSocket as GqlWebSocket;
use async_graphql::http::WebSocketProtocols;
use async_graphql::http::WsMessage;
use async_graphql::http::default_on_connection_init;
use async_graphql::http::default_on_ping;
use futures_util::SinkExt as _;
use futures_util::StreamExt as _;
use http::HeaderValue;
use http::StatusCode;
use http::header;
use hyper_util::rt::TokioIo;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::Role;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// `GraphQL` WebSocket subscription responder (`GraphQL` over WebSocket).
///
/// Usage in a handler:
///
/// ```ignore
/// let schema = Schema::build(QueryRoot, MutationRoot, SubscriptionRoot).finish();
/// router.route(Method::GET, "/ws", move |req: Request| {
///     let schema = schema.clone();
///     async move { GraphQLSubscription::new(req, schema) }
/// });
/// ```
#[cfg(not(feature = "compio"))]
pub struct GraphQLSubscription<E, OnConnInit = DefaultOnConnInitType, OnPing = DefaultOnPingType>
where
  E: Executor,
{
  request: Request,
  executor: E,
  data: Data,
  on_connection_init: OnConnInit,
  on_ping: OnPing,
  keepalive_timeout: Option<Duration>,
}

#[cfg(not(feature = "compio"))]
impl<E> GraphQLSubscription<E, DefaultOnConnInitType, DefaultOnPingType>
where
  E: Executor,
{
  pub fn new(request: Request, executor: E) -> Self {
    Self {
      request,
      executor,
      data: Data::default(),
      on_connection_init: default_on_connection_init,
      on_ping: default_on_ping,
      keepalive_timeout: None,
    }
  }
}

#[cfg(not(feature = "compio"))]
impl<E, OnConnInit, OnPing> GraphQLSubscription<E, OnConnInit, OnPing>
where
  E: Executor,
{
  pub fn with_data(mut self, data: Data) -> Self {
    self.data = data;
    self
  }

  pub fn keepalive_timeout(mut self, timeout: impl Into<Option<Duration>>) -> Self {
    self.keepalive_timeout = timeout.into();
    self
  }

  pub fn on_connection_init<F, Fut>(self, f: F) -> GraphQLSubscription<E, F, OnPing>
  where
    F: FnOnce(serde_json::Value) -> Fut + Send + 'static,
    Fut: Future<Output = GqlResult<Data>> + Send + 'static,
  {
    GraphQLSubscription {
      request: self.request,
      executor: self.executor,
      data: self.data,
      on_connection_init: f,
      on_ping: self.on_ping,
      keepalive_timeout: self.keepalive_timeout,
    }
  }

  pub fn on_ping<F, Fut>(self, f: F) -> GraphQLSubscription<E, OnConnInit, F>
  where
    F: FnOnce(Option<&Data>, Option<serde_json::Value>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = GqlResult<Option<serde_json::Value>>> + Send + 'static,
  {
    GraphQLSubscription {
      request: self.request,
      executor: self.executor,
      data: self.data,
      on_connection_init: self.on_connection_init,
      on_ping: f,
      keepalive_timeout: self.keepalive_timeout,
    }
  }
}

#[cfg(not(feature = "compio"))]
impl<E, OnConnInit, OnConnInitFut, OnPing, OnPingFut> Responder
  for GraphQLSubscription<E, OnConnInit, OnPing>
where
  E: Executor + Send + Sync + Clone + 'static,
  OnConnInit: FnOnce(serde_json::Value) -> OnConnInitFut + Send + 'static,
  OnConnInitFut: Future<Output = GqlResult<Data>> + Send + 'static,
  OnPing: FnOnce(Option<&Data>, Option<serde_json::Value>) -> OnPingFut + Clone + Send + 'static,
  OnPingFut: Future<Output = GqlResult<Option<serde_json::Value>>> + Send + 'static,
{
  fn into_response(self) -> Response {
    // Rebuild so we can grab OnUpgrade
    let (parts, body) = self.request.into_parts();
    let req = http::Request::from_parts(parts, body);

    // Parse and negotiate subprotocol
    let selected_protocol = req
      .headers()
      .get(header::SEC_WEBSOCKET_PROTOCOL)
      .and_then(|v| v.to_str().ok())
      .and_then(|protocols| {
        protocols
          .split(',')
          .find_map(|p| WebSocketProtocols::from_str(p.trim()).ok())
      });

    let Some(protocol) = selected_protocol else {
      return (
        StatusCode::BAD_REQUEST,
        "Missing or invalid Sec-WebSocket-Protocol",
      )
        .into_response();
    };

    // Compute accept key
    let Some(key) = req.headers().get("Sec-WebSocket-Key") else {
      return (
        StatusCode::BAD_REQUEST,
        "Missing Sec-WebSocket-Key for WebSocket upgrade",
      )
        .into_response();
    };

    let accept = {
      use base64::Engine as _;
      use base64::engine::general_purpose::STANDARD;
      use sha1::Digest;
      use sha1::Sha1;
      let mut sha1 = Sha1::new();
      sha1.update(key.as_bytes());
      sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
      STANDARD.encode(sha1.finalize())
    };

    // Build upgrade response
    let builder = http::Response::builder()
      .status(StatusCode::SWITCHING_PROTOCOLS)
      .header(header::UPGRADE, "websocket")
      .header(header::CONNECTION, "Upgrade")
      .header("Sec-WebSocket-Accept", accept)
      .header(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(protocol.sec_websocket_protocol()),
      );

    let response = builder.body(TakoBody::empty()).unwrap();

    // Upgrade and run GraphQL WS server
    if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
      let executor = self.executor.clone();
      let data = self.data;
      let on_conn_init = self.on_connection_init;
      let on_ping = self.on_ping;
      let keepalive = self.keepalive_timeout;

      tokio::spawn(async move {
        if let Ok(upgraded) = on_upgrade.await {
          let upgraded = TokioIo::new(upgraded);
          let ws = WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await;
          let (mut sink, stream) = ws.split();

          let input = stream
            .take_while(|res| futures_util::future::ready(res.is_ok()))
            .map(Result::unwrap)
            .filter_map(|msg| match msg {
              tokio_tungstenite::tungstenite::Message::Text(_)
              | tokio_tungstenite::tungstenite::Message::Binary(_) => {
                futures_util::future::ready(Some(msg))
              }
              _ => futures_util::future::ready(None),
            })
            .map(tokio_tungstenite::tungstenite::Message::into_data);

          let mut stream = GqlWebSocket::new(executor, input, protocol)
            .connection_data(data)
            .on_connection_init(on_conn_init)
            .on_ping(on_ping.clone())
            .keepalive_timeout(keepalive)
            .map(|msg| match msg {
              WsMessage::Text(text) => tokio_tungstenite::tungstenite::Message::Text(text.into()),
              WsMessage::Close(_code, _status) => {
                // tungstenite CloseFrame conversion requires CloseCode; close without reason
                tokio_tungstenite::tungstenite::Message::Close(None)
              }
            });

          while let Some(item) = stream.next().await {
            if sink.send(item).await.is_err() {
              break;
            }
          }
        }
      });
    }

    response
  }
}
