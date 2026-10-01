//! Graceful-shutdown broadcast that connections check with one atomic load.

use std::sync::Mutex;
use std::sync::MutexGuard;
use std::sync::PoisonError;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::task::Waker;

/// Tells a server's HTTP/1 connections to shut down gracefully.
///
/// A connection registers its task once and afterwards only loads a flag, so
/// watching for shutdown takes no lock on the request path.
#[derive(Default)]
pub struct ShutdownSignal {
  stopping: AtomicBool,
  slots: Mutex<Slots>,
}

#[derive(Default)]
struct Slots {
  wakers: Vec<Option<Waker>>,
  free: Vec<usize>,
}

impl ShutdownSignal {
  /// Marks the signal as triggered and wakes every registered connection.
  pub fn trigger(&self) {
    self.stopping.store(true, Ordering::Release);
    for waker in self.lock().wakers.iter().flatten() {
      waker.wake_by_ref();
    }
  }

  pub fn is_triggered(&self) -> bool {
    self.stopping.load(Ordering::Acquire)
  }

  pub(super) fn register(&self, waker: &Waker) -> usize {
    let mut slots = self.lock();
    if let Some(slot) = slots.free.pop() {
      slots.wakers[slot] = Some(waker.clone());
      slot
    } else {
      slots.wakers.push(Some(waker.clone()));
      slots.wakers.len() - 1
    }
  }

  pub(super) fn update(&self, slot: usize, waker: &Waker) {
    self.lock().wakers[slot] = Some(waker.clone());
  }

  pub(super) fn deregister(&self, slot: usize) {
    let mut slots = self.lock();
    slots.wakers[slot] = None;
    slots.free.push(slot);
  }

  #[cfg(test)]
  pub(super) fn registered(&self) -> usize {
    self.lock().wakers.iter().flatten().count()
  }

  fn lock(&self) -> MutexGuard<'_, Slots> {
    self.slots.lock().unwrap_or_else(PoisonError::into_inner)
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;
  use std::sync::atomic::AtomicUsize;
  use std::sync::atomic::Ordering;
  use std::task::Wake;
  use std::task::Waker;

  use super::ShutdownSignal;

  #[derive(Default)]
  struct CountingWaker(AtomicUsize);

  impl Wake for CountingWaker {
    fn wake(self: Arc<Self>) {
      self.0.fetch_add(1, Ordering::SeqCst);
    }
  }

  fn count(waker: &Arc<CountingWaker>) -> usize {
    waker.0.load(Ordering::SeqCst)
  }

  #[test]
  fn trigger_wakes_each_registered_connection_once() {
    let signal = ShutdownSignal::default();
    let (gone, live) = (
      Arc::new(CountingWaker::default()),
      Arc::new(CountingWaker::default()),
    );
    let gone_slot = signal.register(&Waker::from(gone.clone()));
    signal.register(&Waker::from(live.clone()));
    signal.deregister(gone_slot);
    assert!(!signal.is_triggered());
    signal.trigger();
    assert!(signal.is_triggered());
    assert_eq!((count(&gone), count(&live)), (0, 1));
  }

  #[test]
  fn updated_waker_replaces_the_registered_one() {
    let signal = ShutdownSignal::default();
    let (old, new) = (
      Arc::new(CountingWaker::default()),
      Arc::new(CountingWaker::default()),
    );
    let slot = signal.register(&Waker::from(old.clone()));
    signal.update(slot, &Waker::from(new.clone()));
    signal.trigger();
    assert_eq!((count(&old), count(&new)), (0, 1));
  }

  #[test]
  fn freed_slots_are_reused() {
    let signal = ShutdownSignal::default();
    let waker = Waker::from(Arc::new(CountingWaker::default()));
    let first = signal.register(&waker);
    signal.deregister(first);
    assert_eq!(signal.register(&waker), first);
  }
}
