//! The future that runs an HTTP/1 connection under the server's shutdown
//! signal and header deadline.

use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::Arc;
use std::task::Context;
use std::task::Poll;
use std::task::Waker;
use std::time::Duration;

use pin_project_lite::pin_project;
use tokio::time::Instant;
use tokio::time::Sleep;

use super::ConnCtx;
use super::ShutdownSignal;

pin_project! {
  /// Runs an HTTP/1 connection: starts its graceful shutdown once the server's
  /// [`ShutdownSignal`] fires, and closes it when no request head arrives
  /// within the header deadline.
  ///
  /// The deadline is checked on a timer that ticks every half deadline, so an
  /// idle connection closes between one and one and a half deadlines after its
  /// last request, with no timer work per request. Resolves to `None` when the
  /// deadline closed the connection.
  pub struct ConnDriver<C, G, H> {
    #[pin]
    conn: C,
    graceful: Option<G>,
    watch: Watch,
    #[pin]
    deadline: Option<Deadline<H>>,
  }
}

impl<C, G, H> ConnDriver<C, G, H>
where
  C: Future,
  G: FnOnce(Pin<&mut C>),
  H: Deref<Target = ConnCtx>,
{
  /// Wraps `conn`; a `header_timeout` of `None` disables the header deadline.
  pub fn new(
    conn: C,
    graceful: G,
    signal: Arc<ShutdownSignal>,
    ctx: H,
    header_timeout: Option<Duration>,
  ) -> Self {
    Self {
      conn,
      graceful: Some(graceful),
      watch: Watch {
        signal,
        slot: None,
        waker: None,
      },
      deadline: header_timeout.map(|timeout| Deadline::new(ctx, timeout)),
    }
  }
}

impl<C, G, H> Future for ConnDriver<C, G, H>
where
  C: Future,
  G: FnOnce(Pin<&mut C>),
  H: Deref<Target = ConnCtx>,
{
  type Output = Option<C::Output>;

  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let mut this = self.project();
    let new_waker = this.watch.observe(cx);
    if this.watch.signal.is_triggered()
      && let Some(graceful) = this.graceful.take()
    {
      graceful(this.conn.as_mut());
    }
    if let Some(deadline) = this.deadline.as_mut().as_pin_mut()
      && deadline.expired(cx, new_waker)
    {
      return Poll::Ready(None);
    }
    this.conn.poll(cx).map(Some)
  }
}

struct Watch {
  signal: Arc<ShutdownSignal>,
  slot: Option<usize>,
  waker: Option<Waker>,
}

impl Watch {
  /// Keeps the current task registered; true when its waker is new.
  fn observe(&mut self, cx: &Context<'_>) -> bool {
    let current = cx.waker();
    match (self.slot, &self.waker) {
      (Some(_), Some(waker)) if waker.will_wake(current) => return false,
      (Some(slot), _) => self.signal.update(slot, current),
      (None, _) => self.slot = Some(self.signal.register(current)),
    }
    self.waker = Some(current.clone());
    true
  }
}

impl Drop for Watch {
  fn drop(&mut self) {
    if let Some(slot) = self.slot.take() {
      self.signal.deregister(slot);
    }
  }
}

pin_project! {
  struct Deadline<H> {
    #[pin]
    sleep: Sleep,
    tick: Duration,
    ctx: H,
    seen: u32,
    idle_ticks: u8,
  }
}

impl<H: Deref<Target = ConnCtx>> Deadline<H> {
  fn new(ctx: H, timeout: Duration) -> Self {
    let tick = timeout / 2;
    Self {
      sleep: tokio::time::sleep(tick),
      tick,
      ctx,
      seen: 0,
      idle_ticks: 0,
    }
  }

  /// Advances the tick timer; true once the connection stayed idle, with no
  /// new request head, for two ticks in a row.
  ///
  /// The timer is polled only when it elapsed or the task's waker is new,
  /// since it keeps the last registered waker.
  fn expired(self: Pin<&mut Self>, cx: &mut Context<'_>, new_waker: bool) -> bool {
    let mut this = self.project();
    if !new_waker && !this.sleep.is_elapsed() {
      return false;
    }
    while this.sleep.as_mut().poll(cx).is_ready() {
      let requests = this.ctx.activity.requests();
      if this.ctx.activity.is_busy() || requests != *this.seen {
        *this.seen = requests;
        *this.idle_ticks = 0;
      } else {
        *this.idle_ticks += 1;
        if *this.idle_ticks == 2 {
          return true;
        }
      }
      this.sleep.as_mut().reset(Instant::now() + *this.tick);
    }
    false
  }
}

#[cfg(test)]
mod tests {
  use std::future::Future;
  use std::net::SocketAddr;
  use std::pin::Pin;
  use std::sync::Arc;
  use std::task::Context;
  use std::task::Poll;
  use std::time::Duration;

  use tokio::time::Instant;

  use super::ConnDriver;
  use crate::router::Router;
  use crate::server_support::ConnCtx;
  use crate::server_support::ShutdownSignal;

  #[derive(Default)]
  struct FakeConn {
    closing: bool,
  }

  impl Future for FakeConn {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<()> {
      if self.closing {
        Poll::Ready(())
      } else {
        Poll::Pending
      }
    }
  }

  fn close(conn: Pin<&mut FakeConn>) {
    conn.get_mut().closing = true;
  }

  fn ctx() -> Arc<ConnCtx> {
    let peer: SocketAddr = "127.0.0.1:4000".parse().unwrap();
    Arc::new(ConnCtx::new(Arc::new(Router::new()), peer))
  }

  type TestDriver = ConnDriver<FakeConn, fn(Pin<&mut FakeConn>), Arc<ConnCtx>>;

  fn driver(
    signal: &Arc<ShutdownSignal>,
    ctx: &Arc<ConnCtx>,
    deadline: Option<Duration>,
  ) -> TestDriver {
    ConnDriver::new(
      FakeConn::default(),
      close as fn(Pin<&mut FakeConn>),
      Arc::clone(signal),
      Arc::clone(ctx),
      deadline,
    )
  }

  #[tokio::test]
  async fn trigger_starts_graceful_shutdown_of_an_idle_connection() {
    let signal = Arc::new(ShutdownSignal::default());
    let task = tokio::spawn(driver(&signal, &ctx(), None));
    tokio::task::yield_now().await;
    signal.trigger();
    let output = tokio::time::timeout(Duration::from_secs(1), task)
      .await
      .unwrap()
      .unwrap();
    assert_eq!(output, Some(()));
  }

  #[tokio::test]
  async fn shutdown_triggered_before_the_first_poll_is_observed() {
    let signal = Arc::new(ShutdownSignal::default());
    signal.trigger();
    let output = tokio::time::timeout(Duration::from_secs(1), driver(&signal, &ctx(), None))
      .await
      .unwrap();
    assert_eq!(output, Some(()));
  }

  #[tokio::test(start_paused = true)]
  async fn silent_connection_closes_at_the_header_deadline() {
    let signal = Arc::new(ShutdownSignal::default());
    let start = Instant::now();
    let output = driver(&signal, &ctx(), Some(Duration::from_secs(30))).await;
    assert_eq!(output, None);
    assert_eq!(start.elapsed(), Duration::from_secs(30));
  }

  #[tokio::test(start_paused = true)]
  async fn busy_connection_outlives_the_header_deadline() {
    let signal = Arc::new(ShutdownSignal::default());
    let ctx = ctx();
    ctx.activity.request_started();
    let task = tokio::spawn(driver(&signal, &ctx, Some(Duration::from_secs(30))));
    tokio::time::sleep(Duration::from_secs(300)).await;
    assert!(!task.is_finished());
    ctx.activity.request_finished();
    assert_eq!(task.await.unwrap(), None);
  }

  #[tokio::test(start_paused = true)]
  async fn each_request_restarts_the_header_deadline() {
    let signal = Arc::new(ShutdownSignal::default());
    let ctx = ctx();
    let task = tokio::spawn(driver(&signal, &ctx, Some(Duration::from_secs(30))));
    tokio::time::sleep(Duration::from_secs(20)).await;
    ctx.activity.request_started();
    ctx.activity.request_finished();
    let idle_since = Instant::now();
    tokio::time::sleep(Duration::from_secs(29)).await;
    assert!(!task.is_finished());
    assert_eq!(task.await.unwrap(), None);
    let idle = idle_since.elapsed();
    assert!(
      idle >= Duration::from_secs(30) && idle < Duration::from_secs(45),
      "{idle:?}"
    );
  }

  #[tokio::test]
  async fn finished_connections_release_their_shutdown_slot() {
    let signal = Arc::new(ShutdownSignal::default());
    let ctx = ctx();
    let mut first = Box::pin(driver(&signal, &ctx, None));
    assert!(futures_util::poll!(first.as_mut()).is_pending());
    drop(first);
    let mut second = Box::pin(driver(&signal, &ctx, None));
    assert!(futures_util::poll!(second.as_mut()).is_pending());
    assert_eq!(signal.registered(), 1);
  }
}
