//! Quinn's cancel-safe reads keep the receive stream available for `STOP_SENDING` even after
//! a pending body read. h3-quinn 0.0.10 instead moves it into an outstanding read future.

mod streams;

use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use bytes::Bytes;
use futures_util::Stream;
use futures_util::StreamExt;
use futures_util::stream;
use h3::error::Code;
use h3::quic;
use h3::quic::ConnectionErrorIncoming;
use h3::quic::StreamErrorIncoming;
pub(crate) use streams::BidiStream;
pub(crate) use streams::RecvStream;
pub(crate) use streams::SendStream;

type Streams<T> = Pin<Box<dyn Stream<Item = Result<T, quinn::ConnectionError>> + Send>>;
type QuinnBidi = (quinn::SendStream, quinn::RecvStream);

pub(crate) struct Connection {
  conn: quinn::Connection,
  incoming_bi: Streams<QuinnBidi>,
  incoming_uni: Streams<quinn::RecvStream>,
  opening_bi: Streams<QuinnBidi>,
  opening_uni: Streams<quinn::SendStream>,
}

impl Connection {
  pub(crate) fn new(connection: quinn::Connection) -> Self {
    Self {
      conn: connection.clone(),
      incoming_bi: Box::pin(stream::unfold(connection.clone(), |conn| async {
        Some((conn.accept_bi().await, conn))
      })),
      incoming_uni: Box::pin(stream::unfold(connection.clone(), |conn| async {
        Some((conn.accept_uni().await, conn))
      })),
      opening_bi: Box::pin(stream::unfold(connection.clone(), |conn| async {
        Some((conn.open_bi().await, conn))
      })),
      opening_uni: Box::pin(stream::unfold(connection, |conn| async {
        Some((conn.open_uni().await, conn))
      })),
    }
  }
}

impl quic::Connection<Bytes> for Connection {
  type RecvStream = RecvStream;
  type OpenStreams = Self;

  fn poll_accept_recv(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Self::RecvStream, ConnectionErrorIncoming>> {
    let stream = ready!(self.incoming_uni.poll_next_unpin(cx))
      .expect("accept loop is unbounded")
      .map_err(connection_error)?;
    Poll::Ready(Ok(RecvStream(stream)))
  }

  fn poll_accept_bidi(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Self::BidiStream, ConnectionErrorIncoming>> {
    let streams = ready!(self.incoming_bi.poll_next_unpin(cx))
      .expect("accept loop is unbounded")
      .map_err(connection_error)?;
    Poll::Ready(Ok(BidiStream::new(streams)))
  }

  fn opener(&self) -> Self::OpenStreams {
    Self::new(self.conn.clone())
  }
}

impl quic::OpenStreams<Bytes> for Connection {
  type BidiStream = BidiStream;
  type SendStream = SendStream;

  fn poll_open_bidi(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Self::BidiStream, StreamErrorIncoming>> {
    let streams = ready!(self.opening_bi.poll_next_unpin(cx))
      .expect("open loop is unbounded")
      .map_err(stream_connection_error)?;
    Poll::Ready(Ok(BidiStream::new(streams)))
  }

  fn poll_open_send(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Self::SendStream, StreamErrorIncoming>> {
    let stream = ready!(self.opening_uni.poll_next_unpin(cx))
      .expect("open loop is unbounded")
      .map_err(stream_connection_error)?;
    Poll::Ready(Ok(SendStream::new(stream)))
  }

  fn close(&mut self, code: Code, reason: &[u8]) {
    self.conn.close(
      code
        .value()
        .try_into()
        .expect("HTTP/3 error code fits QUIC"),
      reason,
    );
  }
}

#[cfg(feature = "webtransport")]
impl h3_datagram::quic_traits::DatagramConnectionExt<Bytes> for Connection {
  type SendDatagramHandler = h3_quinn::datagram::SendDatagramHandler;
  type RecvDatagramHandler = h3_quinn::datagram::RecvDatagramHandler;

  fn send_datagram_handler(&self) -> Self::SendDatagramHandler {
    <h3_quinn::Connection as h3_datagram::quic_traits::DatagramConnectionExt<Bytes>>::send_datagram_handler(
      &h3_quinn::Connection::new(self.conn.clone()),
    )
  }

  fn recv_datagram_handler(&self) -> Self::RecvDatagramHandler {
    <h3_quinn::Connection as h3_datagram::quic_traits::DatagramConnectionExt<Bytes>>::recv_datagram_handler(
      &h3_quinn::Connection::new(self.conn.clone()),
    )
  }
}

fn connection_error(error: quinn::ConnectionError) -> ConnectionErrorIncoming {
  match error {
    quinn::ConnectionError::ApplicationClosed(close) => ConnectionErrorIncoming::ApplicationClose {
      error_code: close.error_code.into_inner(),
    },
    quinn::ConnectionError::TimedOut => ConnectionErrorIncoming::Timeout,
    error => ConnectionErrorIncoming::Undefined(Arc::new(error)),
  }
}

fn stream_connection_error(error: quinn::ConnectionError) -> StreamErrorIncoming {
  StreamErrorIncoming::ConnectionErrorIncoming {
    connection_error: connection_error(error),
  }
}

fn read_error(error: quinn::ReadError) -> StreamErrorIncoming {
  match error {
    quinn::ReadError::Reset(code) => StreamErrorIncoming::StreamTerminated {
      error_code: code.into_inner(),
    },
    quinn::ReadError::ConnectionLost(error) => stream_connection_error(error),
    error => StreamErrorIncoming::Unknown(Box::new(error)),
  }
}

fn write_error(error: quinn::WriteError) -> StreamErrorIncoming {
  match error {
    quinn::WriteError::Stopped(code) => StreamErrorIncoming::StreamTerminated {
      error_code: code.into_inner(),
    },
    quinn::WriteError::ConnectionLost(error) => stream_connection_error(error),
    error => StreamErrorIncoming::Unknown(Box::new(error)),
  }
}
