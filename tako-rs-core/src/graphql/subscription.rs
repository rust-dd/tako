use std::future::Future;
use std::time::Duration;

use async_graphql::Data;
use async_graphql::Executor;
use async_graphql::Result as GqlResult;
use async_graphql::http::DefaultOnConnInitType;
use async_graphql::http::DefaultOnPingType;
use async_graphql::http::default_on_connection_init;
use async_graphql::http::default_on_ping;
use http::HeaderValue;
use http::StatusCode;
use http::header;
use hyper::upgrade::OnUpgrade;

use super::GraphQLWebSocket;
use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;
use crate::ws;

/// Upgrades the request and serves `GraphQL` subscriptions over the negotiated subprotocol.
///
/// # Examples
///
/// ```ignore
/// let schema = Schema::build(QueryRoot, MutationRoot, SubscriptionRoot).finish();
/// router.route(Method::GET, "/ws", move |req: Request| {
///     let schema = schema.clone();
///     async move { GraphQLSubscription::new(req, schema) }
/// });
/// ```
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
    let accept = match ws::accept(&self.request) {
      Ok(accept) => accept,
      Err(status) => return ws::rejection(status),
    };
    let protocol = match super::protocol::negotiate(self.request.headers()) {
      Ok(super::GraphQLProtocol(protocol)) => protocol,
      Err(rejection) => return rejection.into_response(),
    };
    let response = http::Response::builder()
      .status(StatusCode::SWITCHING_PROTOCOLS)
      .header(header::UPGRADE, "websocket")
      .header(header::CONNECTION, "Upgrade")
      .header(header::SEC_WEBSOCKET_ACCEPT, accept)
      .header(
        header::SEC_WEBSOCKET_PROTOCOL,
        HeaderValue::from_static(protocol.sec_websocket_protocol()),
      )
      .body(TakoBody::empty())
      .expect("valid upgrade response");
    let Self {
      mut request,
      executor,
      data,
      on_connection_init,
      on_ping,
      keepalive_timeout,
    } = self;
    if let Some(on_upgrade) = request.extensions_mut().remove::<OnUpgrade>() {
      ws::spawn_session(on_upgrade, move |socket| {
        GraphQLWebSocket::new(socket, executor, protocol)
          .with_data(data)
          .on_connection_init(on_connection_init)
          .on_ping(on_ping)
          .keepalive_timeout(keepalive_timeout)
          .serve()
      });
    }
    response
  }
}
