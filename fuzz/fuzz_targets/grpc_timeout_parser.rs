#![no_main]

//! Fuzz target: `parse_grpc_timeout`.
//!
//! Feeds arbitrary UTF-8 input through `parse_grpc_timeout`. Invalid input
//! must return `None` rather than panicking.

use libfuzzer_sys::fuzz_target;
use tako_core::grpc::parse_grpc_timeout;

fuzz_target!(|data: &[u8]| {
  if let Ok(s) = std::str::from_utf8(data) {
    let _ = parse_grpc_timeout(s);
  }
});
