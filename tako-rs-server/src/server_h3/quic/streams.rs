use std::future::Future;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::ready;

use bytes::Buf;
use bytes::Bytes;
use h3::quic;
use h3::quic::StreamErrorIncoming;
use h3::quic::StreamId;
use h3::quic::WriteBuf;

pub(crate) struct RecvStream(pub(super) quinn::RecvStream);

impl quic::RecvStream for RecvStream {
  type Buf = Bytes;

  fn poll_data(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Option<Bytes>, StreamErrorIncoming>> {
    let chunk = ready!(std::pin::pin!(self.0.read_chunk(usize::MAX, true)).poll(cx));
    Poll::Ready(
      chunk
        .map(|chunk| chunk.map(|chunk| chunk.bytes))
        .map_err(super::read_error),
    )
  }

  fn stop_sending(&mut self, error_code: u64) {
    let _ = self
      .0
      .stop(error_code.try_into().expect("HTTP/3 error code fits QUIC"));
  }

  fn recv_id(&self) -> StreamId {
    u64::from(self.0.id())
      .try_into()
      .expect("QUIC stream id fits HTTP/3")
  }
}

pub(crate) struct SendStream {
  stream: quinn::SendStream,
  pending: Option<WriteBuf<Bytes>>,
}

impl SendStream {
  pub(super) fn new(stream: quinn::SendStream) -> Self {
    Self {
      stream,
      pending: None,
    }
  }
}

impl quic::SendStream<Bytes> for SendStream {
  fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), StreamErrorIncoming>> {
    if let Some(data) = &mut self.pending {
      while data.has_remaining() {
        let written = ready!(Pin::new(&mut self.stream).poll_write(cx, data.chunk()))
          .map_err(super::write_error)?;
        data.advance(written);
      }
    }
    self.pending = None;
    Poll::Ready(Ok(()))
  }

  fn send_data<T: Into<WriteBuf<Bytes>>>(&mut self, data: T) -> Result<(), StreamErrorIncoming> {
    if self.pending.is_some() {
      return Err(StreamErrorIncoming::ConnectionErrorIncoming {
        connection_error: quic::ConnectionErrorIncoming::InternalError(
          "HTTP/3 write is already pending".into(),
        ),
      });
    }
    self.pending = Some(data.into());
    Ok(())
  }

  fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), StreamErrorIncoming>> {
    ready!(self.poll_ready(cx))?;
    Poll::Ready(
      self
        .stream
        .finish()
        .map_err(|error| StreamErrorIncoming::Unknown(Box::new(error))),
    )
  }

  fn reset(&mut self, reset_code: u64) {
    let _ = self
      .stream
      .reset(reset_code.try_into().expect("HTTP/3 error code fits QUIC"));
  }

  fn send_id(&self) -> StreamId {
    u64::from(self.stream.id())
      .try_into()
      .expect("QUIC stream id fits HTTP/3")
  }
}

impl quic::SendStreamUnframed<Bytes> for SendStream {
  fn poll_send<D: Buf>(
    &mut self,
    cx: &mut Context<'_>,
    buf: &mut D,
  ) -> Poll<Result<usize, StreamErrorIncoming>> {
    ready!(quic::SendStream::poll_ready(self, cx))?;
    let written =
      ready!(Pin::new(&mut self.stream).poll_write(cx, buf.chunk())).map_err(super::write_error)?;
    buf.advance(written);
    Poll::Ready(Ok(written))
  }
}

pub(crate) struct BidiStream {
  send: SendStream,
  recv: RecvStream,
}

impl BidiStream {
  pub(super) fn new((send, recv): (quinn::SendStream, quinn::RecvStream)) -> Self {
    Self {
      send: SendStream::new(send),
      recv: RecvStream(recv),
    }
  }
}

impl quic::BidiStream<Bytes> for BidiStream {
  type SendStream = SendStream;
  type RecvStream = RecvStream;

  fn split(self) -> (SendStream, RecvStream) {
    (self.send, self.recv)
  }
}

impl quic::RecvStream for BidiStream {
  type Buf = Bytes;

  fn poll_data(
    &mut self,
    cx: &mut Context<'_>,
  ) -> Poll<Result<Option<Bytes>, StreamErrorIncoming>> {
    self.recv.poll_data(cx)
  }

  fn stop_sending(&mut self, error_code: u64) {
    self.recv.stop_sending(error_code);
  }

  fn recv_id(&self) -> StreamId {
    self.recv.recv_id()
  }
}

impl quic::SendStream<Bytes> for BidiStream {
  fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), StreamErrorIncoming>> {
    self.send.poll_ready(cx)
  }

  fn send_data<T: Into<WriteBuf<Bytes>>>(&mut self, data: T) -> Result<(), StreamErrorIncoming> {
    self.send.send_data(data)
  }

  fn poll_finish(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), StreamErrorIncoming>> {
    self.send.poll_finish(cx)
  }

  fn reset(&mut self, reset_code: u64) {
    self.send.reset(reset_code);
  }

  fn send_id(&self) -> StreamId {
    self.send.send_id()
  }
}

impl quic::SendStreamUnframed<Bytes> for BidiStream {
  fn poll_send<D: Buf>(
    &mut self,
    cx: &mut Context<'_>,
    buf: &mut D,
  ) -> Poll<Result<usize, StreamErrorIncoming>> {
    self.send.poll_send(cx, buf)
  }
}
