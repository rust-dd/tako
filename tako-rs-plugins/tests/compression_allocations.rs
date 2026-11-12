#![cfg(feature = "plugins")]

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;

use bytes::Bytes;
use futures_util::FutureExt;
use http::header::CONTENT_ENCODING;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_plugins::plugins::compression::CompressionBuilder;

struct CountingAllocator;

thread_local! {
  static ALLOCATIONS: Cell<Option<(usize, usize)>> = const { Cell::new(None) };
}

fn count(bytes: usize) {
  let _ = ALLOCATIONS.try_with(|cell| {
    if let Some((calls, total)) = cell.get() {
      cell.set(Some((calls + 1, total + bytes)));
    }
  });
}

// Delegation preserves System's allocation and deallocation contracts.
unsafe impl GlobalAlloc for CountingAllocator {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    count(layout.size());
    unsafe { System.alloc(layout) }
  }

  unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
    count(layout.size());
    unsafe { System.alloc_zeroed(layout) }
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
    unsafe { System.dealloc(ptr, layout) }
  }

  unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    count(new_size);
    unsafe { System.realloc(ptr, layout, new_size) }
  }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn measure<T>(f: impl FnOnce() -> T) -> (T, (usize, usize)) {
  ALLOCATIONS.set(Some((0, 0)));
  let result = f();
  let counts = ALLOCATIONS.replace(None).unwrap();
  (result, counts)
}

fn request(accepted: &str) -> Request {
  http::Request::builder()
    .header("accept-encoding", accepted)
    .body(TakoBody::empty())
    .unwrap()
}

#[test]
fn negotiation_does_not_allocate_for_each_accept_encoding_token() {
  let mut router = Router::new();
  router.plugin(CompressionBuilder::new().build());
  router.get("/", || async { Bytes::from_static(b"response") });
  router.dispatch(request("")).now_or_never().unwrap();
  let baseline_request = request("");
  let (_, baseline) = measure(|| router.dispatch(baseline_request).now_or_never().unwrap());

  for accepted in ["identity".into(), "unknown, ".repeat(1024)] {
    let request = request(&accepted);
    let (response, actual) = measure(|| router.dispatch(request).now_or_never().unwrap());
    assert!(!response.headers().contains_key(CONTENT_ENCODING));
    assert_eq!(
      response
        .into_body()
        .collect()
        .now_or_never()
        .unwrap()
        .unwrap()
        .to_bytes(),
      "response"
    );
    eprintln!(
      "header bytes={}, empty header allocations={baseline:?}, actual={actual:?}",
      accepted.len()
    );
    assert_eq!(
      actual, baseline,
      "negotiation should borrow coding names instead of allocating per token"
    );
  }
}

#[test]
fn inspecting_a_buffered_body_below_min_size_does_not_allocate() {
  let mut router = Router::new();
  router.plugin(CompressionBuilder::new().build());
  router.get("/", || async {
    http::Response::builder()
      .header("content-type", "application/json")
      .body(TakoBody::from(Bytes::from_static(b"response")))
      .unwrap()
  });
  router.dispatch(request("")).now_or_never().unwrap();
  let baseline_request = request("");
  let (_, baseline) = measure(|| router.dispatch(baseline_request).now_or_never().unwrap());
  let request = request("gzip");
  let (response, actual) = measure(|| router.dispatch(request).now_or_never().unwrap());
  assert!(!response.headers().contains_key(CONTENT_ENCODING));
  assert_eq!(
    response
      .into_body()
      .collect()
      .now_or_never()
      .unwrap()
      .unwrap()
      .to_bytes(),
    "response"
  );
  eprintln!("identity allocations={baseline:?}, inspected body={actual:?}");
  assert_eq!(
    actual, baseline,
    "a single body frame does not need an allocated chunk queue"
  );
}
