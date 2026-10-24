use std::io;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use pin_project_lite::pin_project;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::ReadBuf;

pin_project! {
  /// A connection's IO plus what it holds for its whole life, such as a `max_connections`
  /// permit; the lease survives an HTTP upgrade, so upgraded sockets stay counted.
  pub struct Leased<S, L> {
    #[pin]
    io: S,
    _lease: L,
  }
}

impl<S, L> Leased<S, L> {
  pub fn new(io: S, lease: L) -> Self {
    Self { io, _lease: lease }
  }
}

impl<S: AsyncRead, L> AsyncRead for Leased<S, L> {
  #[inline]
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>,
  ) -> Poll<io::Result<()>> {
    self.project().io.poll_read(cx, buf)
  }
}

impl<S: AsyncWrite, L> AsyncWrite for Leased<S, L> {
  #[inline]
  fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
    self.project().io.poll_write(cx, buf)
  }

  #[inline]
  fn poll_write_vectored(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[io::IoSlice<'_>],
  ) -> Poll<io::Result<usize>> {
    self.project().io.poll_write_vectored(cx, bufs)
  }

  #[inline]
  fn is_write_vectored(&self) -> bool {
    self.io.is_write_vectored()
  }

  #[inline]
  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    self.project().io.poll_flush(cx)
  }

  #[inline]
  fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    self.project().io.poll_shutdown(cx)
  }
}

impl<S: hyper::rt::Read, L> hyper::rt::Read for Leased<S, L> {
  #[inline]
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: hyper::rt::ReadBufCursor<'_>,
  ) -> Poll<io::Result<()>> {
    self.project().io.poll_read(cx, buf)
  }
}

impl<S: hyper::rt::Write, L> hyper::rt::Write for Leased<S, L> {
  #[inline]
  fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
    self.project().io.poll_write(cx, buf)
  }

  #[inline]
  fn poll_write_vectored(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    bufs: &[io::IoSlice<'_>],
  ) -> Poll<io::Result<usize>> {
    self.project().io.poll_write_vectored(cx, bufs)
  }

  #[inline]
  fn is_write_vectored(&self) -> bool {
    self.io.is_write_vectored()
  }

  #[inline]
  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    self.project().io.poll_flush(cx)
  }

  #[inline]
  fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
    self.project().io.poll_shutdown(cx)
  }
}

#[cfg(test)]
mod tests {
  use std::io::IoSlice;
  use std::sync::Arc;

  use tokio::io::AsyncReadExt;
  use tokio::io::AsyncWrite;
  use tokio::io::AsyncWriteExt;
  use tokio::net::TcpListener;
  use tokio::net::TcpStream;
  use tokio::sync::Semaphore;

  use super::Leased;

  #[tokio::test]
  async fn holds_the_lease_until_the_io_drops() {
    let semaphore = Arc::new(Semaphore::new(1));
    let permit = Arc::clone(&semaphore).acquire_owned().await.unwrap();
    let (io, _peer) = tokio::io::duplex(64);
    let leased = Leased::new(io, permit);
    assert_eq!(semaphore.available_permits(), 0);
    drop(leased);
    assert_eq!(semaphore.available_permits(), 1);
  }

  #[tokio::test]
  async fn forwards_reads_and_writes() {
    let (io, mut peer) = tokio::io::duplex(64);
    let mut leased = Leased::new(io, ());
    let mut buf = [0; 4];
    leased.write_all(b"ping").await.unwrap();
    peer.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"ping");
    peer.write_all(b"pong").await.unwrap();
    leased.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"pong");
  }

  #[tokio::test]
  async fn keeps_the_vectored_writes_of_a_tcp_stream() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let client = TcpStream::connect(listener.local_addr().unwrap())
      .await
      .unwrap();
    let (mut server, _) = listener.accept().await.unwrap();
    let mut leased = Leased::new(client, ());
    assert!(leased.is_write_vectored());
    let parts = [IoSlice::new(b"he"), IoSlice::new(b"llo")];
    assert_eq!(leased.write_vectored(&parts).await.unwrap(), 5);
    let mut buf = [0; 5];
    server.read_exact(&mut buf).await.unwrap();
    assert_eq!(&buf, b"hello");
  }
}
