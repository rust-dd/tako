use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;

use hyper::rt::Read;
use hyper::rt::ReadBufCursor;
use hyper::rt::Write;
use pin_project_lite::pin_project;

use super::ConnActivity;

pin_project! {
  /// Observes HTTP/1 flushes so the next header deadline cannot cut off a response.
  pub struct Http1Io<S> {
    #[pin]
    io: S,
    activity: Arc<ConnActivity>,
  }
}

impl<S> Http1Io<S> {
  pub fn new(io: S, activity: Arc<ConnActivity>) -> Self {
    Self { io, activity }
  }
}

impl<S: Read> Read for Http1Io<S> {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: ReadBufCursor<'_>,
  ) -> Poll<io::Result<()>> {
    self.project().io.poll_read(cx, buf)
  }
}

impl<S: Write> Write for Http1Io<S> {
  fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
    self.project().io.poll_write(cx, buf)
  }

  fn poll_write_vectored(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[io::IoSlice<'_>],
  ) -> Poll<io::Result<usize>> {
    self.project().io.poll_write_vectored(cx, bufs)
  }

  fn is_write_vectored(&self) -> bool {
    self.io.is_write_vectored()
  }

  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    let this = self.project();
    let result = this.io.poll_flush(cx);
    if matches!(result, Poll::Ready(Ok(()))) {
      this.activity.response_flushed();
    }
    result
  }

  fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    self.project().io.poll_shutdown(cx)
  }
}
