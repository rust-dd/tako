#![cfg(feature = "async-graphql")]

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;

use async_graphql::http::MultipartOptions;
use bytes::Bytes;
use futures_util::FutureExt;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::graphql::receive_graphql_batch;

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

#[test]
fn buffered_json_avoids_a_second_body_sized_allocation() {
  for query_size in [1024, 64 * 1024, 512 * 1024] {
    let body = Bytes::from(format!(r#"{{"query":"{}"}}"#, "a".repeat(query_size)));
    let mut req = http::Request::builder()
      .method("POST")
      .header("content-type", "application/json")
      .body(TakoBody::from(body.clone()))
      .unwrap();
    let baseline_body = TakoBody::from(body.clone());
    let (_, baseline) = measure(|| {
      async {
        let bytes = http_body_util::Limited::new(baseline_body, 4 * 1024 * 1024)
          .collect()
          .await
          .unwrap()
          .to_bytes();
        async_graphql::http::receive_batch_body(
          Some("application/json"),
          futures_util::io::Cursor::new(bytes),
          MultipartOptions::default(),
        )
        .await
        .unwrap()
      }
      .now_or_never()
      .unwrap()
    });
    let (parsed, actual) = measure(|| {
      receive_graphql_batch(&mut req, MultipartOptions::default())
        .now_or_never()
        .unwrap()
        .unwrap()
    });
    assert_eq!(parsed.into_single().unwrap().query.len(), query_size);
    eprintln!(
      "JSON bytes={}, before calls={} bytes={}, after calls={} bytes={}",
      body.len(),
      baseline.0,
      baseline.1,
      actual.0,
      actual.1
    );
    assert!(
      baseline.1.saturating_sub(actual.1) >= body.len(),
      "the already-buffered JSON body was copied again"
    );
  }
}
