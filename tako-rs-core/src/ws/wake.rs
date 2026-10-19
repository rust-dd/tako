//! Fans one transport wake-up out to every task waiting on it; a transport keeps a single waker.

use std::sync::Arc;
use std::sync::Mutex;
use std::sync::PoisonError;
use std::task::Context;
use std::task::Wake;
use std::task::Waker;

use smallvec::SmallVec;

#[derive(Default)]
struct Waiting(Mutex<SmallVec<[Waker; 2]>>);

impl Wake for Waiting {
  fn wake(self: Arc<Self>) {
    self.wake_by_ref();
  }

  fn wake_by_ref(self: &Arc<Self>) {
    let waiting = std::mem::take(&mut *self.0.lock().unwrap_or_else(PoisonError::into_inner));
    for waker in waiting {
      waker.wake();
    }
  }
}

pub(crate) struct Fanout {
  waiting: Arc<Waiting>,
  waker: Waker,
}

impl Fanout {
  pub(crate) fn new() -> Self {
    let waiting = Arc::new(Waiting::default());
    let waker = Waker::from(Arc::clone(&waiting));
    Self { waiting, waker }
  }

  /// Adds the task of `cx` to the waiters; poll the transport with the returned waker.
  pub(crate) fn register(&self, cx: &Context<'_>) -> &Waker {
    let mut waiting = self
      .waiting
      .0
      .lock()
      .unwrap_or_else(PoisonError::into_inner);
    if !waiting.iter().any(|waker| waker.will_wake(cx.waker())) {
      waiting.push(cx.waker().clone());
    }
    &self.waker
  }

  pub(crate) fn wake_all(&self) {
    self.waker.wake_by_ref();
  }
}
