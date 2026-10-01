//! Type-erased handler future that small handlers run without a heap allocation.

use std::future::Future;
use std::marker::PhantomData;
use std::marker::PhantomPinned;
use std::mem::MaybeUninit;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use futures_util::future::BoxFuture;

use crate::types::Response;

/// Inline capacity in words: enough for handlers that take no extractors and
/// capture little, such as `|| async { "ok" }`.
const WORDS: usize = 6;

type Slot = MaybeUninit<[usize; WORDS]>;

/// A handler's response future, stored inline when it fits in [`WORDS`] words
/// and boxed otherwise.
///
/// It is `!Unpin`, so once pinned the inline future never moves again; moving
/// it before the first poll is allowed like for any future.
pub(crate) struct HandlerFuture {
  repr: Repr,
  _pinned: PhantomPinned,
}

enum Repr {
  Inline { slot: Slot, vtable: &'static VTable },
  Boxed(BoxFuture<'static, Response>),
}

struct VTable {
  poll: unsafe fn(*mut (), &mut Context<'_>) -> Poll<Response>,
  drop: unsafe fn(*mut ()),
}

struct Table<F>(PhantomData<F>);

impl<F: Future<Output = Response>> Table<F> {
  const VTABLE: VTable = VTable {
    poll: poll_inline::<F>,
    drop: drop_inline::<F>,
  };
}

impl HandlerFuture {
  /// Stores `future` inline when it fits, boxing it otherwise. `Send` is
  /// required here because the inline slot hides the future's type.
  pub(crate) fn new<F>(future: F) -> Self
  where
    F: Future<Output = Response> + Send + 'static,
  {
    let repr = if size_of::<F>() <= size_of::<Slot>() && align_of::<F>() <= align_of::<Slot>() {
      let mut slot = Slot::uninit();
      // SAFETY: the slot is large and aligned enough for `F`, checked above.
      unsafe { slot.as_mut_ptr().cast::<F>().write(future) };
      Repr::Inline {
        slot,
        vtable: &Table::<F>::VTABLE,
      }
    } else {
      Repr::Boxed(Box::pin(future))
    };
    Self {
      repr,
      _pinned: PhantomPinned,
    }
  }

  #[cfg(test)]
  pub(crate) fn is_inline(&self) -> bool {
    matches!(self.repr, Repr::Inline { .. })
  }
}

impl Future for HandlerFuture {
  type Output = Response;

  #[inline]
  fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Response> {
    // SAFETY: nothing is moved out of `self`; an inline future stays pinned in
    // its slot because `HandlerFuture` is `!Unpin`.
    let this = unsafe { self.get_unchecked_mut() };
    match &mut this.repr {
      // SAFETY: the slot holds the live future that `new` wrote for this vtable.
      Repr::Inline { slot, vtable } => unsafe { (vtable.poll)(slot.as_mut_ptr().cast(), cx) },
      Repr::Boxed(future) => future.as_mut().poll(cx),
    }
  }
}

impl Drop for HandlerFuture {
  fn drop(&mut self) {
    if let Repr::Inline { slot, vtable } = &mut self.repr {
      // SAFETY: the slot holds the live future that `new` wrote; this is its only drop.
      unsafe { (vtable.drop)(slot.as_mut_ptr().cast()) }
    }
  }
}

/// # Safety
///
/// `future` must point at a live `F` that stays pinned in place.
unsafe fn poll_inline<F: Future<Output = Response>>(
  future: *mut (),
  cx: &mut Context<'_>,
) -> Poll<Response> {
  // SAFETY: guaranteed by the caller.
  unsafe { Pin::new_unchecked(&mut *future.cast::<F>()) }.poll(cx)
}

/// # Safety
///
/// `future` must point at a live `F` that is not used afterwards.
unsafe fn drop_inline<F>(future: *mut ()) {
  // SAFETY: guaranteed by the caller.
  unsafe { future.cast::<F>().drop_in_place() }
}

#[cfg(test)]
mod tests {
  use std::future::Future;
  use std::pin::pin;
  use std::sync::Arc;
  use std::sync::atomic::AtomicUsize;
  use std::sync::atomic::Ordering;
  use std::task::Context;
  use std::task::Poll;
  use std::task::Waker;

  use super::HandlerFuture;
  use crate::body::TakoBody;
  use crate::types::Response;

  struct DropCounter(Arc<AtomicUsize>);

  impl Drop for DropCounter {
    fn drop(&mut self) {
      self.0.fetch_add(1, Ordering::SeqCst);
    }
  }

  #[repr(align(64))]
  struct Aligned([u8; 8]);

  impl Aligned {
    fn sum(&self) -> u8 {
      self.0.iter().sum()
    }
  }

  fn ok() -> Response {
    Response::new(TakoBody::from("ok"))
  }

  fn poll_once(future: std::pin::Pin<&mut HandlerFuture>) -> Poll<Response> {
    future.poll(&mut Context::from_waker(Waker::noop()))
  }

  #[test]
  fn small_future_is_stored_inline_and_completes() {
    let future = HandlerFuture::new(async { ok() });
    assert!(future.is_inline());
    let mut future = pin!(future);
    assert!(matches!(poll_once(future.as_mut()), Poll::Ready(resp) if resp.status() == 200));
  }

  #[test]
  fn large_future_is_boxed_and_completes() {
    let payload = [7u8; 512];
    let future = HandlerFuture::new(async move {
      std::hint::black_box(&payload);
      ok()
    });
    assert!(!future.is_inline());
    let mut future = pin!(future);
    assert!(poll_once(future.as_mut()).is_ready());
  }

  #[test]
  fn over_aligned_future_is_boxed() {
    let aligned = Aligned([1; 8]);
    let future = HandlerFuture::new(async move {
      std::hint::black_box(aligned.sum());
      ok()
    });
    assert!(!future.is_inline());
  }

  #[test]
  fn pending_inline_future_resumes() {
    let mut yielded = false;
    let future = HandlerFuture::new(std::future::poll_fn(move |cx| {
      if yielded {
        Poll::Ready(ok())
      } else {
        yielded = true;
        cx.waker().wake_by_ref();
        Poll::Pending
      }
    }));
    assert!(future.is_inline());
    let mut future = pin!(future);
    assert!(poll_once(future.as_mut()).is_pending());
    assert!(poll_once(future.as_mut()).is_ready());
  }

  #[test]
  fn inline_future_state_is_dropped_exactly_once() {
    let drops = Arc::new(AtomicUsize::new(0));
    let counter = DropCounter(Arc::clone(&drops));
    drop(HandlerFuture::new(async move {
      let _counter = &counter;
      ok()
    }));
    assert_eq!(drops.load(Ordering::SeqCst), 1);

    let counter = DropCounter(Arc::clone(&drops));
    let mut future = Box::pin(HandlerFuture::new(async move {
      let _counter = &counter;
      ok()
    }));
    assert!(poll_once(future.as_mut()).is_ready());
    drop(future);
    assert_eq!(drops.load(Ordering::SeqCst), 2);
  }
}
