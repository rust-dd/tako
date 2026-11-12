#![cfg(feature = "grpc")]

use std::alloc::GlobalAlloc;
use std::alloc::Layout;
use std::alloc::System;
use std::cell::Cell;

use bytes::Bytes;
use futures_util::FutureExt;
use prost::Message;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::BodyLimit;
use tako_rs_core::grpc::GrpcRequest;

struct CountingAllocator;

thread_local! {
  static ALLOCATIONS: Cell<Option<usize>> = const { Cell::new(None) };
}

fn count() {
  let _ = ALLOCATIONS.try_with(|cell| {
    if let Some(calls) = cell.get() {
      cell.set(Some(calls + 1));
    }
  });
}

// Delegation preserves System's allocation and deallocation contracts.
unsafe impl GlobalAlloc for CountingAllocator {
  unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
    count();
    unsafe { System.alloc(layout) }
  }

  unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
    unsafe { System.dealloc(ptr, layout) }
  }

  unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
    count();
    unsafe { System.realloc(ptr, layout, new_size) }
  }
}

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

#[derive(Clone, PartialEq, Message)]
struct Number {
  #[prost(int32, tag = "1")]
  value: i32,
}

#[test]
fn parameter_free_unary_extraction_avoids_content_type_allocations() {
  for content_type in [
    "application/grpc",
    "application/grpc+proto",
    "APPLICATION/GRPC",
    "Application/Grpc+Proto",
  ] {
    let mut request = http::Request::builder()
      .method("POST")
      .header("content-type", content_type)
      .body(TakoBody::from(Bytes::from_static(&[0, 0, 0, 0, 2, 8, 42])))
      .unwrap();
    request.extensions_mut().insert(BodyLimit(None));

    ALLOCATIONS.set(Some(0));
    let result = GrpcRequest::<Number>::from_request(&mut request).now_or_never();
    let allocations = ALLOCATIONS.replace(None).unwrap();

    assert_eq!(result.unwrap().unwrap().message.value, 42);
    assert_eq!(allocations, 0, "{content_type}");
  }
}
