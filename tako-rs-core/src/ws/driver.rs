//! Runs a connection's handler, writes its output once per wake-up and closes the connection.

use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use futures_util::FutureExt;
use futures_util::future::CatchUnwind;

use super::CloseCode;
use super::runtime::Shared;
use super::runtime::Sleep;
use super::runtime::lock;
use super::runtime::sleep;
use super::socket::Core;

// A peer that stops reading cannot keep a closing connection's task alive longer than this.
const CLOSE_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, PartialEq, Eq)]
enum Phase {
  Running,
  Detached,
  Closing,
  Done,
}

pub(crate) struct Driver<F> {
  shared: Shared<Core>,
  handler: Option<Pin<Box<CatchUnwind<AssertUnwindSafe<F>>>>>,
  lifetime: Option<Sleep>,
  close_timer: Option<Sleep>,
  phase: Phase,
}

impl<F: Future<Output = ()>> Driver<F> {
  pub(crate) fn new(shared: Shared<Core>, handler: F, max_lifetime: Option<Duration>) -> Self {
    Self {
      shared,
      handler: Some(Box::pin(AssertUnwindSafe(handler).catch_unwind())),
      lifetime: max_lifetime.map(sleep),
      close_timer: None,
      phase: Phase::Running,
    }
  }

  fn close(&mut self, code: CloseCode) {
    self.handler = None;
    {
      let mut core = lock(&self.shared);
      core.close_if_open(code);
      core.finished = true;
      core.wake_waiting();
    }
    self.close_timer = Some(sleep(CLOSE_TIMEOUT));
    self.phase = Phase::Closing;
  }

  fn lifetime_over(&mut self, cx: &mut Context<'_>) -> bool {
    self
      .lifetime
      .as_mut()
      .is_some_and(|lifetime| lifetime.as_mut().poll(cx).is_ready())
  }

  fn poll_handler(&mut self, cx: &mut Context<'_>) -> Poll<()> {
    let Some(handler) = self.handler.as_mut() else {
      self.phase = Phase::Detached;
      return Poll::Ready(());
    };
    lock(&self.shared).owner.enter();
    let result = handler.as_mut().poll(cx);
    {
      let mut core = lock(&self.shared);
      core.owner.leave();
      let _ = core.poll_write_out(cx);
    }
    match result {
      Poll::Pending => return Poll::Pending,
      Poll::Ready(Ok(())) => {
        self.handler = None;
        self.phase = Phase::Detached;
      }
      Poll::Ready(Err(_)) => self.close(CloseCode::INTERNAL),
    }
    Poll::Ready(())
  }

  fn poll_detached(&mut self, cx: &mut Context<'_>) -> Poll<()> {
    {
      let mut core = lock(&self.shared);
      if core.handles > 0 {
        core.driver = Some(cx.waker().clone());
        return Poll::Pending;
      }
    }
    self.close(CloseCode::NORMAL);
    Poll::Ready(())
  }

  fn poll_closing(&mut self, cx: &mut Context<'_>) -> Poll<()> {
    if self
      .close_timer
      .as_mut()
      .is_none_or(|timer| timer.as_mut().poll(cx).is_ready())
    {
      self.phase = Phase::Done;
      return Poll::Ready(());
    }
    let mut core = lock(&self.shared);
    let done = match core.poll_write_out(cx) {
      Poll::Pending => return Poll::Pending,
      Poll::Ready(Err(_)) => true,
      Poll::Ready(Ok(())) => core.poll_shutdown(cx).is_ready(),
    };
    if !done {
      return Poll::Pending;
    }
    drop(core);
    self.phase = Phase::Done;
    Poll::Ready(())
  }
}

impl<F: Future<Output = ()>> Future for Driver<F> {
  type Output = ();

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
    let this = &mut *self;
    loop {
      if matches!(this.phase, Phase::Running | Phase::Detached) && this.lifetime_over(cx) {
        this.close(CloseCode::GOING_AWAY);
      }
      let step = match this.phase {
        Phase::Running => this.poll_handler(cx),
        Phase::Detached => this.poll_detached(cx),
        Phase::Closing => this.poll_closing(cx),
        Phase::Done => return Poll::Ready(()),
      };
      if step.is_pending() {
        return Poll::Pending;
      }
    }
  }
}

#[cfg(test)]
mod tests;
