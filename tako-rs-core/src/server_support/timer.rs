use std::future::Future;
use std::pin::Pin;
use std::ptr;
use std::sync::Arc;
use std::sync::atomic::AtomicPtr;
use std::sync::atomic::Ordering;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;
use std::time::Instant;

use hyper::rt::Sleep;
use hyper::rt::Timer;

/// hyper timer that reuses one Tokio sleep for the lifetime of a connection.
///
/// hyper drops its header-read sleep after every request head and asks for a
/// new one when the next head starts. A fresh Tokio sleep per request costs a
/// timer registration and removal, both under the time driver's global lock,
/// which caps a busy multi-threaded runtime. Parking the dropped sleep keeps it
/// registered, and moving it to the next, later deadline is a lock-free update.
#[derive(Default)]
pub struct ConnectionTimer {
  slot: Arc<SleepSlot>,
}

impl ConnectionTimer {
  /// Creates a timer for one connection.
  #[must_use]
  pub fn new() -> Self {
    Self::default()
  }
}

impl Timer for ConnectionTimer {
  fn sleep(&self, duration: Duration) -> Pin<Box<dyn Sleep>> {
    self.sleep_until(self.now() + duration)
  }

  fn sleep_until(&self, deadline: Instant) -> Pin<Box<dyn Sleep>> {
    let deadline = tokio::time::Instant::from_std(deadline);
    let sleep = match self.slot.take() {
      Some(mut sleep) => {
        sleep.as_mut().reset(deadline);
        sleep
      }
      None => Box::pin(tokio::time::sleep_until(deadline)),
    };
    Box::pin(RecycledSleep {
      sleep: Some(sleep),
      slot: Arc::clone(&self.slot),
    })
  }

  fn reset(&self, sleep: &mut Pin<Box<dyn Sleep>>, new_deadline: Instant) {
    match sleep.as_mut().downcast_mut_pin::<RecycledSleep>() {
      Some(recycled) => recycled.reset(new_deadline),
      None => *sleep = self.sleep_until(new_deadline),
    }
  }

  fn now(&self) -> Instant {
    tokio::time::Instant::now().into_std()
  }
}

struct RecycledSleep {
  sleep: Option<Pin<Box<tokio::time::Sleep>>>,
  slot: Arc<SleepSlot>,
}

impl RecycledSleep {
  fn reset(self: Pin<&mut Self>, deadline: Instant) {
    if let Some(sleep) = &mut self.get_mut().sleep {
      sleep
        .as_mut()
        .reset(tokio::time::Instant::from_std(deadline));
    }
  }
}

impl Future for RecycledSleep {
  type Output = ();

  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
    match &mut self.get_mut().sleep {
      Some(sleep) => sleep.as_mut().poll(cx),
      None => Poll::Ready(()),
    }
  }
}

impl Sleep for RecycledSleep {}

impl Drop for RecycledSleep {
  fn drop(&mut self) {
    if let Some(sleep) = self.sleep.take() {
      self.slot.put(sleep);
    }
  }
}

/// Lock-free single-entry cache for a pinned Tokio sleep.
#[derive(Default)]
struct SleepSlot(AtomicPtr<tokio::time::Sleep>);

impl SleepSlot {
  fn take(&self) -> Option<Pin<Box<tokio::time::Sleep>>> {
    let ptr = self.0.swap(ptr::null_mut(), Ordering::Acquire);
    // SAFETY: a non-null pointer was leaked from a pinned box by `put`; the swap
    // hands its ownership to this call and the sleep never moved.
    (!ptr.is_null()).then(|| Box::into_pin(unsafe { Box::from_raw(ptr) }))
  }

  fn put(&self, sleep: Pin<Box<tokio::time::Sleep>>) {
    // SAFETY: the box is leaked in place and only ever re-pinned by `take`.
    let ptr = Box::into_raw(unsafe { Pin::into_inner_unchecked(sleep) });
    let previous = self.0.swap(ptr, Ordering::AcqRel);
    if !previous.is_null() {
      // SAFETY: the swap transferred ownership of the previous pinned entry.
      drop(Box::into_pin(unsafe { Box::from_raw(previous) }));
    }
  }
}

impl Drop for SleepSlot {
  fn drop(&mut self) {
    drop(self.take());
  }
}

#[cfg(test)]
mod tests {
  use std::pin::Pin;
  use std::time::Duration;

  use futures_util::future::poll_immediate;
  use hyper::rt::Sleep;
  use hyper::rt::Timer;

  use super::ConnectionTimer;
  use super::RecycledSleep;

  fn tokio_sleep_addr(sleep: &mut Pin<Box<dyn Sleep>>) -> *const tokio::time::Sleep {
    let recycled = sleep
      .as_mut()
      .downcast_mut_pin::<RecycledSleep>()
      .expect("ConnectionTimer hands out RecycledSleep");
    let inner = recycled.get_mut().sleep.as_ref().expect("live sleep");
    std::ptr::from_ref::<tokio::time::Sleep>(inner)
  }

  #[tokio::test(start_paused = true)]
  async fn sleep_completes_at_its_deadline() {
    let timer = ConnectionTimer::new();
    let mut sleep = timer.sleep_until(timer.now() + Duration::from_secs(30));
    tokio::time::advance(Duration::from_secs(29)).await;
    assert!(poll_immediate(&mut sleep).await.is_none());
    tokio::time::advance(Duration::from_secs(2)).await;
    assert!(poll_immediate(&mut sleep).await.is_some());
  }

  #[tokio::test(start_paused = true)]
  async fn dropped_sleep_is_reused_for_the_next_deadline() {
    let timer = ConnectionTimer::new();
    let mut first = timer.sleep_until(timer.now() + Duration::from_secs(30));
    assert!(poll_immediate(&mut first).await.is_none());
    let first_addr = tokio_sleep_addr(&mut first);
    drop(first);
    let mut second = timer.sleep_until(timer.now() + Duration::from_secs(30));
    assert_eq!(tokio_sleep_addr(&mut second), first_addr);
  }

  #[tokio::test(start_paused = true)]
  async fn reused_sleep_waits_for_the_new_deadline() {
    let timer = ConnectionTimer::new();
    let start = timer.now();
    let mut first = timer.sleep_until(start + Duration::from_secs(30));
    assert!(poll_immediate(&mut first).await.is_none());
    drop(first);
    let mut second = timer.sleep_until(start + Duration::from_secs(60));
    tokio::time::advance(Duration::from_secs(45)).await;
    assert!(poll_immediate(&mut second).await.is_none());
    tokio::time::advance(Duration::from_secs(16)).await;
    assert!(poll_immediate(&mut second).await.is_some());
  }

  #[tokio::test(start_paused = true)]
  async fn reset_moves_the_deadline() {
    let timer = ConnectionTimer::new();
    let start = timer.now();
    let mut sleep = timer.sleep_until(start + Duration::from_secs(30));
    timer.reset(&mut sleep, start + Duration::from_secs(60));
    tokio::time::advance(Duration::from_secs(45)).await;
    assert!(poll_immediate(&mut sleep).await.is_none());
    tokio::time::advance(Duration::from_secs(16)).await;
    assert!(poll_immediate(&mut sleep).await.is_some());
  }
}
