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
use futures_util::Stream;
use futures_util::StreamExt as _;

use crate::ws::CloseCode;
use crate::ws::CloseFrame;
use crate::ws::Error;
use crate::ws::Message;

/// Runs async-graphql's WebSocket protocol over any `Sink`/`Stream` of WebSocket messages.
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

impl<S, E>
  GraphQLWebSocket<
    futures_util::stream::SplitSink<S, Message>,
    futures_util::stream::SplitStream<S>,
    E,
    DefaultOnConnInitType,
    DefaultOnPingType,
  >
where
  S: Stream<Item = Result<Message, Error>> + Sink<Message>,
  E: Executor,
{
  /// Splits a socket that is both the `Sink` and the `Stream`.
  pub fn new(stream: S, executor: E, protocol: WebSocketProtocols) -> Self {
    let (sink, stream) = stream.split();
    GraphQLWebSocket::new_with_pair(sink, stream, executor, protocol)
  }
}

impl<SinkT, StreamT, E>
  GraphQLWebSocket<SinkT, StreamT, E, DefaultOnConnInitType, DefaultOnPingType>
where
  SinkT: Sink<Message>,
  StreamT: Stream<Item = Result<Message, Error>>,
  E: Executor,
{
  /// Uses a sink and a stream that are already separate.
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

impl<SinkT, StreamT, E, OnConnInit, OnPing> GraphQLWebSocket<SinkT, StreamT, E, OnConnInit, OnPing>
where
  SinkT: Sink<Message>,
  StreamT: Stream<Item = Result<Message, Error>>,
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

impl<SinkT, StreamT, E, OnConnInit, OnConnInitFut, OnPing, OnPingFut>
  GraphQLWebSocket<SinkT, StreamT, E, OnConnInit, OnPing>
where
  SinkT: Sink<Message> + Unpin,
  StreamT: Stream<Item = Result<Message, Error>> + Unpin,
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

  /// Serves the protocol until the connection closes or a read fails.
  pub async fn serve(self) {
    let Self {
      sink,
      stream,
      executor,
      data,
      on_connection_init,
      on_ping,
      protocol,
      keepalive_timeout,
    } = self;
    let input = stream
      .take_while(|message| futures_util::future::ready(message.is_ok()))
      .filter_map(|message| {
        futures_util::future::ready(match message {
          Ok(message @ (Message::Text(_) | Message::Binary(_))) => Some(message.into_data()),
          _ => None,
        })
      });
    let output = GqlWebSocket::new(executor, input, protocol)
      .connection_data(data)
      .on_connection_init(on_connection_init)
      .on_ping(on_ping)
      .keepalive_timeout(keepalive_timeout)
      .map(|message| {
        Ok::<_, SinkT::Error>(match message {
          WsMessage::Text(text) => Message::text(text),
          WsMessage::Close(code, reason) => Message::Close(Some(CloseFrame {
            code: CloseCode::new(code),
            reason: reason.into(),
          })),
        })
      });
    let _ = output.forward(sink).await;
  }
}
