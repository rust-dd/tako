//! Async-GraphQL integration for Tako: extractors, responses, and subscriptions.
//!
//! - GraphQLRequest / GraphQLBatchRequest extractors
//! - GraphQLResponse / GraphQLBatchResponse responders
//! - GraphQLSubscription responder for WebSocket subscriptions
//!
//! Enable via the `async-graphql` cargo feature.
#![cfg(feature = "async-graphql")]
#![cfg_attr(docsrs, doc(cfg(feature = "async-graphql")))]

use std::str::FromStr;
#[cfg(not(feature = "compio"))]
use std::{future::Future, time::Duration};

use async_graphql::{
  BatchRequest as GqlBatchRequest, BatchResponse as GqlBatchResponse,
  http::{MultipartOptions, WebSocketProtocols},
};
#[cfg(not(feature = "compio"))]
use async_graphql::{
  Data, Executor, Result as GqlResult,
  http::{DefaultOnConnInitType, DefaultOnPingType, default_on_connection_init, default_on_ping},
};
#[cfg(not(feature = "compio"))]
use async_graphql::http::{WebSocket as GqlWebSocket, WsMessage};
#[cfg(not(feature = "compio"))]
use futures_util::{Sink, SinkExt as _, Stream, StreamExt as _};
use http::{HeaderValue, StatusCode, header};
use http_body_util::BodyExt;
#[cfg(not(feature = "compio"))]
use hyper_util::rt::TokioIo;
#[cfg(not(feature = "compio"))]
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

use crate::body::TakoBody;
use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
#[cfg(feature = "graphiql")]
pub use crate::graphiql::GraphiQL;
#[cfg(feature = "graphiql")]
pub use crate::graphiql::graphiql;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Single GraphQL request extractor.
pub struct GraphQLRequest(pub async_graphql::Request);

impl GraphQLRequest {
  pub fn into_inner(self) -> async_graphql::Request {
    self.0
  }
}

/// Batch GraphQL request extractor.
pub struct GraphQLBatchRequest(pub GqlBatchRequest);

impl GraphQLBatchRequest {
  pub fn into_inner(self) -> GqlBatchRequest {
    self.0
  }
}

/// Errors that can occur while parsing GraphQL HTTP requests.
#[derive(Debug)]
pub enum GraphQLError {
  MissingQuery,
  BodyRead(String),
  InvalidJson(String),
  Parse(String),
}

/// Per-request or global options for GraphQL extraction.
#[derive(Clone)]
pub struct GraphQLOptions {
  pub multipart: MultipartOptions,
}

impl Default for GraphQLOptions {
  fn default() -> Self {
    Self {
      multipart: MultipartOptions::default(),
    }
  }
}

impl Responder for GraphQLError {
  fn into_response(self) -> Response {
    match self {
      GraphQLError::MissingQuery => {
        (StatusCode::BAD_REQUEST, "Missing GraphQL query").into_response()
      }
      GraphQLError::BodyRead(e) => {
        (StatusCode::BAD_REQUEST, format!("Failed to read body: {e}")).into_response()
      }
      GraphQLError::InvalidJson(e) => {
        (StatusCode::BAD_REQUEST, format!("Invalid JSON: {e}")).into_response()
      }
      GraphQLError::Parse(e) => {
        (StatusCode::BAD_REQUEST, format!("Invalid request: {e}")).into_response()
      }
    }
  }
}

/// Extracted WebSocket protocol for GraphQL subscriptions.
pub struct GraphQLProtocol(pub WebSocketProtocols);

#[derive(Debug)]
pub struct GraphQLProtocolRejection;

impl Responder for GraphQLProtocolRejection {
  fn into_response(self) -> Response {
    (
      StatusCode::BAD_REQUEST,
      "Missing or invalid Sec-WebSocket-Protocol",
    )
      .into_response()
  }
}

impl<'a> FromRequestParts<'a> for GraphQLProtocol {
  type Error = GraphQLProtocolRejection;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(|protocols| {
          protocols
            .split(',')
            .find_map(|p| WebSocketProtocols::from_str(p.trim()).ok())
        })
        .map(GraphQLProtocol)
        .ok_or(GraphQLProtocolRejection),
    )
  }
}

impl<'a> FromRequest<'a> for GraphQLProtocol {
  type Error = GraphQLProtocolRejection;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(|protocols| {
          protocols
            .split(',')
            .find_map(|p| WebSocketProtocols::from_str(p.trim()).ok())
        })
        .map(GraphQLProtocol)
        .ok_or(GraphQLProtocolRejection),
    )
  }
}

#[inline]
fn resolve_opts(req: &Request) -> MultipartOptions {
  // Prefer per-request options in extensions
  if let Some(opts) = req.extensions().get::<GraphQLOptions>() {
    return opts.multipart.clone();
  }
  // Fallback to global state
  if let Some(global) = crate::state::get_state::<GraphQLOptions>() {
    return global.as_ref().multipart.clone();
  }
  MultipartOptions::default()
}

fn parse_get_request(req: &Request) -> Result<async_graphql::Request, GraphQLError> {
  let qs = req.uri().query().unwrap_or("");
  async_graphql::http::parse_query_string(qs).map_err(|e| GraphQLError::Parse(e.to_string()))
}

async fn read_body_bytes(req: &mut Request) -> Result<bytes::Bytes, GraphQLError> {
  req
    .body_mut()
    .collect()
    .await
    .map_err(|e| GraphQLError::BodyRead(e.to_string()))
    .map(|collected| collected.to_bytes())
}

impl<'a> FromRequest<'a> for GraphQLRequest {
  type Error = GraphQLError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.method() == http::Method::GET {
        return Ok(GraphQLRequest(parse_get_request(req)?));
      }

      // Resolve MultipartOptions: request extensions -> global state -> default
      let opts = resolve_opts(req);

      let body = read_body_bytes(req).await?;
      let content_type = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());

      let reader = futures_util::io::Cursor::new(body.to_vec());
      let req = async_graphql::http::receive_body(content_type.as_deref(), reader, opts)
        .await
        .map_err(|e| GraphQLError::Parse(e.to_string()))?;
      Ok(GraphQLRequest(req))
    }
  }
}

/// Helper to receive a single GraphQL request with custom MultipartOptions.
/// Attach per-request GraphQL options into request extensions.
pub fn attach_graphql_options(req: &mut Request, opts: GraphQLOptions) {
  req.extensions_mut().insert(opts);
}

/// Set global GraphQL options via Tako's global state.
pub fn set_global_graphql_options(opts: GraphQLOptions) {
  crate::state::set_state::<GraphQLOptions>(opts);
}

pub async fn receive_graphql(
  req: &mut Request,
  opts: MultipartOptions,
) -> Result<async_graphql::Request, GraphQLError> {
  if req.method() == http::Method::GET {
    return parse_get_request(req);
  }
  let body = read_body_bytes(req).await?;
  let content_type = req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(|s| s.to_string());
  let reader = futures_util::io::Cursor::new(body.to_vec());
  async_graphql::http::receive_body(content_type.as_deref(), reader, opts)
    .await
    .map_err(|e| GraphQLError::Parse(e.to_string()))
}

/// Helper to receive a batch GraphQL request with custom MultipartOptions.
pub async fn receive_graphql_batch(
  req: &mut Request,
  opts: MultipartOptions,
) -> Result<GqlBatchRequest, GraphQLError> {
  if req.method() == http::Method::GET {
    let single = parse_get_request(req)?;
    return Ok(GqlBatchRequest::Single(single));
  }
  let body = read_body_bytes(req).await?;
  let content_type = req
    .headers()
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(|s| s.to_string());
  let reader = futures_util::io::Cursor::new(body.to_vec());
  async_graphql::http::receive_batch_body(content_type.as_deref(), reader, opts)
    .await
    .map_err(|e| GraphQLError::Parse(e.to_string()))
}

impl<'a> FromRequest<'a> for GraphQLBatchRequest {
  type Error = GraphQLError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.method() == http::Method::GET {
        // Treat GET as single request
        let single = parse_get_request(req)?;
        return Ok(GraphQLBatchRequest(GqlBatchRequest::Single(single)));
      }

      // Resolve MultipartOptions: request extensions -> global state -> default
      let opts = resolve_opts(req);

      let body = read_body_bytes(req).await?;
      let content_type = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
      let reader = futures_util::io::Cursor::new(body.to_vec());
      let batch = async_graphql::http::receive_batch_body(content_type.as_deref(), reader, opts)
        .await
        .map_err(|e| GraphQLError::Parse(e.to_string()))?;
      Ok(GraphQLBatchRequest(batch))
    }
  }
}

/// Single GraphQL response wrapper.
pub struct GraphQLResponse(pub async_graphql::Response);

impl From<async_graphql::Response> for GraphQLResponse {
  fn from(value: async_graphql::Response) -> Self {
    Self(value)
  }
}

impl Responder for GraphQLResponse {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
  }
}

/// Batch GraphQL response wrapper.
pub struct GraphQLBatchResponse(pub GqlBatchResponse);

impl From<GqlBatchResponse> for GraphQLBatchResponse {
  fn from(value: GqlBatchResponse) -> Self {
    Self(value)
  }
}

impl Responder for GraphQLBatchResponse {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
  }
}

/// GraphQL WebSocket subscription responder (GraphQL over WebSocket).
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
    let key = match req.headers().get("Sec-WebSocket-Key") {
      Some(k) => k,
      None => {
        return (
          StatusCode::BAD_REQUEST,
          "Missing Sec-WebSocket-Key for WebSocket upgrade",
        )
          .into_response();
      }
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
            .map(|msg| msg.into_data());

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

/// A generic GraphQL WebSocket driver using an arbitrary Sink/Stream of tungstenite Messages.
///
/// This is a generic API so you can integrate custom websocket
/// transports while reusing Tako's mapping to async-graphql's WebSocket state machine.
#[cfg(not(feature = "compio"))]
pub struct GraphQLWebSocket<SinkT, StreamT, E, OnConnInit, OnPing>
where
  E: Executor,
{
  sink: SinkT,
  stream: StreamT,
  executor: E,
  data: Data,
  on_connection_init: OnConnInit,
  on_ping: OnPing,
  protocol: WebSocketProtocols,
  keepalive_timeout: Option<Duration>,
}

#[cfg(not(feature = "compio"))]
impl<S, E>
  GraphQLWebSocket<
    futures_util::stream::SplitSink<S, tokio_tungstenite::tungstenite::Message>,
    futures_util::stream::SplitStream<S>,
    E,
    DefaultOnConnInitType,
    DefaultOnPingType,
  >
where
  S: Stream<
      Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
    > + Sink<tokio_tungstenite::tungstenite::Message>,
  E: Executor,
{
  /// Create a GraphQLWebSocket from a combined websocket stream implementing Sink+Stream.
  pub fn new(stream: S, executor: E, protocol: WebSocketProtocols) -> Self {
    let (sink, stream) = stream.split();
    GraphQLWebSocket::new_with_pair(sink, stream, executor, protocol)
  }
}

#[cfg(not(feature = "compio"))]
impl<SinkT, StreamT, E>
  GraphQLWebSocket<SinkT, StreamT, E, DefaultOnConnInitType, DefaultOnPingType>
where
  SinkT: Sink<tokio_tungstenite::tungstenite::Message>,
  StreamT: Stream<
    Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
  >,
  E: Executor,
{
  /// Create a GraphQLWebSocket from separate sink and stream.
  pub fn new_with_pair(
    sink: SinkT,
    stream: StreamT,
    executor: E,
    protocol: WebSocketProtocols,
  ) -> Self {
    Self {
      sink,
      stream,
      executor,
      data: Data::default(),
      on_connection_init: default_on_connection_init,
      on_ping: default_on_ping,
      protocol,
      keepalive_timeout: None,
    }
  }
}

#[cfg(not(feature = "compio"))]
impl<SinkT, StreamT, E, OnConnInit, OnPing> GraphQLWebSocket<SinkT, StreamT, E, OnConnInit, OnPing>
where
  SinkT: Sink<tokio_tungstenite::tungstenite::Message>,
  StreamT: Stream<
    Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
  >,
  E: Executor,
{
  pub fn with_data(self, data: Data) -> Self {
    Self { data, ..self }
  }

  pub fn keepalive_timeout(self, timeout: impl Into<Option<Duration>>) -> Self {
    Self {
      keepalive_timeout: timeout.into(),
      ..self
    }
  }
}

#[cfg(not(feature = "compio"))]
impl<SinkT, StreamT, E, OnConnInit, OnConnInitFut, OnPing, OnPingFut>
  GraphQLWebSocket<SinkT, StreamT, E, OnConnInit, OnPing>
where
  SinkT: Sink<tokio_tungstenite::tungstenite::Message> + Unpin,
  StreamT: Stream<
      Item = Result<tokio_tungstenite::tungstenite::Message, tokio_tungstenite::tungstenite::Error>,
    > + Unpin,
  E: Executor,
  OnConnInit: FnOnce(serde_json::Value) -> OnConnInitFut + Send + 'static,
  OnConnInitFut: Future<Output = GqlResult<Data>> + Send + 'static,
  OnPing: FnOnce(Option<&Data>, Option<serde_json::Value>) -> OnPingFut + Clone + Send + 'static,
  OnPingFut: Future<Output = GqlResult<Option<serde_json::Value>>> + Send + 'static,
{
  pub fn on_connection_init<F, Fut>(
    self,
    callback: F,
  ) -> GraphQLWebSocket<SinkT, StreamT, E, F, OnPing>
  where
    F: FnOnce(serde_json::Value) -> Fut + Send + 'static,
    Fut: Future<Output = GqlResult<Data>> + Send + 'static,
  {
    GraphQLWebSocket {
      sink: self.sink,
      stream: self.stream,
      executor: self.executor,
      data: self.data,
      on_connection_init: callback,
      on_ping: self.on_ping,
      protocol: self.protocol,
      keepalive_timeout: self.keepalive_timeout,
    }
  }

  pub fn on_ping<F, Fut>(self, callback: F) -> GraphQLWebSocket<SinkT, StreamT, E, OnConnInit, F>
  where
    F: FnOnce(Option<&Data>, Option<serde_json::Value>) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = GqlResult<Option<serde_json::Value>>> + Send + 'static,
  {
    GraphQLWebSocket {
      sink: self.sink,
      stream: self.stream,
      executor: self.executor,
      data: self.data,
      on_connection_init: self.on_connection_init,
      on_ping: callback,
      protocol: self.protocol,
      keepalive_timeout: self.keepalive_timeout,
    }
  }

  /// Run the GraphQL over WebSocket protocol loop until the connection ends.
  pub async fn serve(mut self) {
    let input = self
      .stream
      .take_while(|res| futures_util::future::ready(res.is_ok()))
      .map(Result::unwrap)
      .filter_map(|msg| match msg {
        tokio_tungstenite::tungstenite::Message::Text(_)
        | tokio_tungstenite::tungstenite::Message::Binary(_) => {
          futures_util::future::ready(Some(msg))
        }
        _ => futures_util::future::ready(None),
      })
      .map(|msg| msg.into_data());

    let mut out_stream = GqlWebSocket::new(self.executor, input, self.protocol)
      .connection_data(self.data)
      .on_connection_init(self.on_connection_init)
      .on_ping(self.on_ping.clone())
      .keepalive_timeout(self.keepalive_timeout)
      .map(|msg| match msg {
        WsMessage::Text(text) => tokio_tungstenite::tungstenite::Message::Text(text.into()),
        WsMessage::Close(_code, _status) => tokio_tungstenite::tungstenite::Message::Close(None),
      });

    while let Some(item) = out_stream.next().await {
      if self.sink.send(item).await.is_err() {
        break;
      }
    }
  }
}
