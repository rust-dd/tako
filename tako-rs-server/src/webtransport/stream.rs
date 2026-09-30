//! WebTransport stream types: thin wrappers over h3's buffered QUIC streams
//! that implement the `tokio::io` and `futures::io` read/write traits.
//!
//! Adapted from `h3-webtransport` 0.1.2 (<https://github.com/hyperium/h3>,
//! MIT License, Copyright (c) 2020 h3 authors).

use std::io;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use bytes::Buf;
use bytes::Bytes;
use h3::quic;
use h3::quic::StreamErrorIncoming;
use h3::stream::BufRecvStream;
use pin_project_lite::pin_project;
use tokio::io::ReadBuf;

use crate::h3_common::connection::QuicConnection;

type QuicBidi = <QuicConnection as quic::OpenStreams<Bytes>>::BidiStream;
type QuicSend = <QuicConnection as quic::OpenStreams<Bytes>>::SendStream;
type QuicRecv = <QuicConnection as quic::Connection<Bytes>>::RecvStream;

pin_project! {
  /// A bidirectional WebTransport stream.
  ///
  /// Implements `AsyncRead` and `AsyncWrite` from both `tokio::io` and
  /// `futures::io`. [`split`](Self::split) it to read and write from
  /// separate tasks; shutting down the write side finishes the stream.
  pub struct BidiStream {
    #[pin]
    stream: BufRecvStream<QuicBidi, Bytes>,
  }
}

pin_project! {
  /// The sending side of a WebTransport stream (`AsyncWrite`).
  pub struct SendStream {
    #[pin]
    stream: BufRecvStream<QuicSend, Bytes>,
  }
}

pin_project! {
  /// The receiving side of a WebTransport stream (`AsyncRead`).
  pub struct RecvStream {
    #[pin]
    stream: BufRecvStream<QuicRecv, Bytes>,
  }
}

impl BidiStream {
  pub(super) fn new(stream: BufRecvStream<QuicBidi, Bytes>) -> Self {
    Self { stream }
  }

  /// Splits the stream into its sending and receiving sides.
  pub fn split(self) -> (SendStream, RecvStream) {
    let (send, recv) = quic::BidiStream::split(self.stream);
    (SendStream { stream: send }, RecvStream { stream: recv })
  }

  pub(super) fn poll_send_header<D: Buf>(
    &mut self,
    cx: &mut Context<'_>,
    buf: &mut D,
  ) -> Poll<Result<usize, StreamErrorIncoming>> {
    quic::SendStreamUnframed::poll_send(&mut self.stream, cx, buf)
  }
}

impl SendStream {
  pub(super) fn new(stream: BufRecvStream<QuicSend, Bytes>) -> Self {
    Self { stream }
  }

  pub(super) fn poll_send_header<D: Buf>(
    &mut self,
    cx: &mut Context<'_>,
    buf: &mut D,
  ) -> Poll<Result<usize, StreamErrorIncoming>> {
    quic::SendStreamUnframed::poll_send(&mut self.stream, cx, buf)
  }
}

impl RecvStream {
  pub(super) fn new(stream: BufRecvStream<QuicRecv, Bytes>) -> Self {
    Self { stream }
  }
}

macro_rules! impl_read {
  ($ty:ty) => {
    impl tokio::io::AsyncRead for $ty {
      fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
      ) -> Poll<io::Result<()>> {
        tokio::io::AsyncRead::poll_read(self.project().stream, cx, buf)
      }
    }

    impl futures_util::io::AsyncRead for $ty {
      fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut [u8],
      ) -> Poll<io::Result<usize>> {
        futures_util::io::AsyncRead::poll_read(self.project().stream, cx, buf)
      }
    }
  };
}

macro_rules! impl_write {
  ($ty:ty) => {
    impl tokio::io::AsyncWrite for $ty {
      fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
      ) -> Poll<io::Result<usize>> {
        tokio::io::AsyncWrite::poll_write(self.project().stream, cx, buf)
      }

      fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        tokio::io::AsyncWrite::poll_flush(self.project().stream, cx)
      }

      fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        tokio::io::AsyncWrite::poll_shutdown(self.project().stream, cx)
      }
    }

    impl futures_util::io::AsyncWrite for $ty {
      fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
      ) -> Poll<io::Result<usize>> {
        futures_util::io::AsyncWrite::poll_write(self.project().stream, cx, buf)
      }

      fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::io::AsyncWrite::poll_flush(self.project().stream, cx)
      }

      fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        futures_util::io::AsyncWrite::poll_close(self.project().stream, cx)
      }
    }
  };
}

impl_read!(BidiStream);
impl_read!(RecvStream);
impl_write!(BidiStream);
impl_write!(SendStream);
