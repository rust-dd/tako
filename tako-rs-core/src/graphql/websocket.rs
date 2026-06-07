//! Generic `GraphQL`-over-WebSocket driver over an arbitrary tungstenite
//! `Sink`/`Stream` pair, reusing async-graphql's WebSocket state machine.

use std::future::Future;
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
use futures_util::Sink;
use futures_util::SinkExt as _;
use futures_util::Stream;
use futures_util::StreamExt as _;

/// A generic `GraphQL` WebSocket driver using an arbitrary `Sink`/`Stream` of tungstenite Messages.
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
  /// Create a `GraphQLWebSocket` from a combined websocket stream implementing `Sink`+`Stream`.
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
  /// Create a `GraphQLWebSocket` from separate sink and stream.
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

  /// Run the `GraphQL` over WebSocket protocol loop until the connection ends.
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
      .map(tokio_tungstenite::tungstenite::Message::into_data);

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
