use std::fmt;
use std::future::poll_fn;
use std::io;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;
use std::task::ready;

use bytes::Buf;
use bytes::BytesMut;
use futures_util::Sink;
use futures_util::Stream;
use hyper::rt::Read;
use hyper::rt::ReadBuf;
use hyper::rt::Write;
use hyper::upgrade::Upgraded;

use super::CloseCode;
use super::CloseFrame;
use super::Error;
use super::Message;
use super::protocol::Protocol;
use super::protocol::State;
use super::receiver::Limits;
use super::receiver::MAX_READ_AHEAD;
use super::runtime::Owner;
use super::runtime::Shared;
use super::runtime::lock;
use super::wake::Fanout;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Config {
  pub limits: Limits,
  pub read_buffer_size: usize,
  pub write_buffer_size: usize,
}

impl Config {
  pub(crate) fn is_valid(self) -> bool {
    // Leave room for a final read chunk and frame headers within one allocation.
    let max = isize::MAX as usize - MAX_READ_AHEAD;
    (1..=MAX_READ_AHEAD).contains(&self.read_buffer_size)
      && self.write_buffer_size <= max
      && self.limits.max_frame_size <= max
      && self.limits.max_message_size <= max
  }
}

impl Default for Config {
  fn default() -> Self {
    Self {
      limits: Limits {
        max_frame_size: 16 << 20,
        max_message_size: 64 << 20,
      },
      read_buffer_size: 4 * 1024,
      write_buffer_size: 64 * 1024,
    }
  }
}

pub(crate) struct Core {
  io: Upgraded,
  read_buf: BytesMut,
  write_buf: BytesMut,
  protocol: Protocol,
  config: Config,
  read_done: bool,
  // Sticky, so a handler that only sends learns of a dead peer on its next send.
  write_failed: Option<io::ErrorKind>,
  read_wake: Fanout,
  write_wake: Fanout,
  pub(crate) owner: Owner,
  pub(crate) handles: usize,
  pub(crate) driver: Option<Waker>,
  pub(crate) finished: bool,
}

impl Core {
  pub(crate) fn new(io: Upgraded, config: Config) -> Self {
    Self {
      io,
      read_buf: BytesMut::with_capacity(config.read_buffer_size),
      write_buf: BytesMut::new(),
      protocol: Protocol::new(config.limits),
      config,
      read_done: false,
      write_failed: None,
      read_wake: Fanout::new(),
      write_wake: Fanout::new(),
      owner: Owner::default(),
      handles: 0,
      driver: None,
      finished: false,
    }
  }

  pub(crate) fn close_if_open(&mut self, code: CloseCode) {
    self.protocol.close_if_open(code);
  }

  pub(crate) fn wake_waiting(&self) {
    self.read_wake.wake_all();
    self.write_wake.wake_all();
  }

  pub(crate) fn poll_recv(&mut self, cx: &mut Context<'_>) -> Poll<Option<Result<Message, Error>>> {
    if self.read_done || self.finished {
      return Poll::Ready(None);
    }
    loop {
      match self.protocol.decode(&mut self.read_buf) {
        Ok(Some(message)) => {
          // Queued now, a pong keeps its place before later replies; a full buffer coalesces it.
          if message.is_ping()
            && (self.write_buf.is_empty() || self.write_buf.len() < self.config.write_buffer_size)
          {
            self.protocol.drain_pong(&mut self.write_buf);
          }
          return Poll::Ready(Some(Ok(message)));
        }
        Ok(None) => {}
        Err(error) => {
          self.protocol.fail(error);
          return self.end(Some(Err(Error::Protocol(error))));
        }
      }
      if matches!(self.protocol.state(), State::CloseReceived | State::Closed) {
        let _ = self.poll_write_out(cx);
        return self.end(None);
      }
      if let Poll::Ready(Err(error)) = self.poll_write_out(cx) {
        return self.end(Some(Err(error)));
      }
      match ready!(self.poll_read_more(cx)) {
        Ok(0) => {
          let reset = self.protocol.eof().err();
          return self.end(reset.map(|error| Err(Error::Protocol(error))));
        }
        Ok(_) => {}
        Err(error) => return self.end(Some(Err(Error::Io(error)))),
      }
    }
  }

  fn end(&mut self, last: Option<Result<Message, Error>>) -> Poll<Option<Result<Message, Error>>> {
    self.read_done = true;
    Poll::Ready(last)
  }

  fn poll_read_more(&mut self, cx: &mut Context<'_>) -> Poll<io::Result<usize>> {
    let want = self
      .protocol
      .wanted(&self.read_buf)
      .min(MAX_READ_AHEAD)
      .max(self.config.read_buffer_size);
    if self.read_buf.is_empty() && self.read_buf.capacity() > MAX_READ_AHEAD {
      self.read_buf = BytesMut::new();
    }
    self.read_buf.reserve(want);
    let mut proxy = Context::from_waker(self.read_wake.register(cx));
    let filled = {
      // Offering only `want` bytes bounds the work of transports that zero-fill the slice.
      let spare = self.read_buf.spare_capacity_mut();
      let len = spare.len().min(want);
      let mut buf = ReadBuf::uninit(&mut spare[..len]);
      ready!(Pin::new(&mut self.io).poll_read(&mut proxy, buf.unfilled()))?;
      buf.filled().len()
    };
    // SAFETY: the transport initialized `filled` bytes of the spare capacity.
    unsafe { self.read_buf.set_len(self.read_buf.len() + filled) };
    Poll::Ready(Ok(filled))
  }

  pub(crate) fn poll_ready_send(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    if let Some(kind) = self.write_failed {
      return Poll::Ready(Err(Error::Io(kind.into())));
    }
    if self.finished {
      return Poll::Ready(Err(Error::Closed));
    }
    if self.write_buf.len() < self.config.write_buffer_size {
      return Poll::Ready(Ok(()));
    }
    match self.poll_write_out(cx) {
      Poll::Pending if self.write_buf.len() >= self.config.write_buffer_size => Poll::Pending,
      Poll::Pending => Poll::Ready(Ok(())),
      ready @ Poll::Ready(_) => ready,
    }
  }

  pub(crate) fn start_send(&mut self, message: Message) -> Result<(), Error> {
    if let Some(kind) = self.write_failed {
      return Err(Error::Io(kind.into()));
    }
    if self.finished {
      return Err(Error::Closed);
    }
    self.protocol.encode(message, &mut self.write_buf)
  }

  pub(crate) fn poll_write_out(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    if let Some(kind) = self.write_failed {
      return Poll::Ready(Err(Error::Io(kind.into())));
    }
    let mut proxy = Context::from_waker(self.write_wake.register(cx));
    let result = loop {
      // A peer that never reads leaves one pong waiting instead of growing the buffer.
      if self.write_buf.is_empty() || self.write_buf.len() < self.config.write_buffer_size {
        self.protocol.drain_pong(&mut self.write_buf);
      }
      self.protocol.drain_close(&mut self.write_buf);
      if self.write_buf.is_empty() {
        break ready!(Pin::new(&mut self.io).poll_flush(&mut proxy));
      }
      match ready!(Pin::new(&mut self.io).poll_write(&mut proxy, &self.write_buf)) {
        Ok(0) => break Err(io::ErrorKind::WriteZero.into()),
        Ok(written) => self.write_buf.advance(written),
        Err(error) => break Err(error),
      }
    };
    if let Err(error) = &result {
      self.write_failed = Some(error.kind());
    }
    Poll::Ready(result.map_err(Error::Io))
  }

  pub(crate) fn poll_shutdown(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    let mut proxy = Context::from_waker(self.write_wake.register(cx));
    ready!(Pin::new(&mut self.io).poll_shutdown(&mut proxy))?;
    Poll::Ready(Ok(()))
  }
}

/// An upgraded WebSocket connection. `send` queues frames; they go out together once the
/// handler waits, when `recv` runs out of input, past `write_buffer_size`, or on `flush`.
pub struct WebSocket {
  shared: Shared<Core>,
}

impl WebSocket {
  pub(crate) fn new(shared: Shared<Core>) -> Self {
    lock(&shared).handles += 1;
    Self { shared }
  }

  /// The next message; `None` once the connection is closed. Cancel-safe.
  pub async fn recv(&mut self) -> Option<Result<Message, Error>> {
    poll_fn(|cx| lock(&self.shared).poll_recv(cx)).await
  }

  /// Queues a message. Called from a task other than the handler's, it writes at once.
  pub async fn send(&mut self, message: Message) -> Result<(), Error> {
    poll_fn(|cx| lock(&self.shared).poll_ready_send(cx)).await?;
    let write_through = {
      let mut core = lock(&self.shared);
      core.start_send(message)?;
      !core.owner.is_current()
    };
    if write_through {
      self.flush().await
    } else {
      Ok(())
    }
  }

  /// Writes every queued frame now.
  pub async fn flush(&mut self) -> Result<(), Error> {
    poll_fn(|cx| lock(&self.shared).poll_write_out(cx)).await
  }

  /// Sends a close frame now, or flushes the reply if the peer already closed.
  /// Repeated calls only flush; the peer's reply still arrives through `recv`.
  pub async fn close(&mut self, frame: Option<CloseFrame>) -> Result<(), Error> {
    lock(&self.shared).start_send(Message::Close(frame))?;
    self.flush().await
  }
}

impl fmt::Debug for WebSocket {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("WebSocket").finish_non_exhaustive()
  }
}

impl Drop for WebSocket {
  fn drop(&mut self) {
    let mut core = lock(&self.shared);
    core.handles -= 1;
    if core.handles == 0
      && let Some(driver) = core.driver.take()
    {
      driver.wake();
    }
  }
}

impl Stream for WebSocket {
  type Item = Result<Message, Error>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    lock(&self.shared).poll_recv(cx)
  }
}

impl Sink<Message> for WebSocket {
  type Error = Error;

  fn poll_ready(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    lock(&self.shared).poll_ready_send(cx)
  }

  fn start_send(self: Pin<&mut Self>, message: Message) -> Result<(), Error> {
    lock(&self.shared).start_send(message)
  }

  fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    lock(&self.shared).poll_write_out(cx)
  }

  fn poll_close(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<(), Error>> {
    let mut core = lock(&self.shared);
    core.close_if_open(CloseCode::NORMAL);
    core.poll_write_out(cx)
  }
}

#[cfg(test)]
mod tests;
