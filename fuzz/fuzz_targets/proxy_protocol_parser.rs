#![no_main]

//! Fuzz target: PROXY protocol v1 + v2 parser.
//!
//! Feeds arbitrary byte input through `tako_server::read_proxy_protocol`
//! against an in-memory cursor. The parser must never panic on malformed
//! input — it should return `Err(io::Error)` instead.

use std::io::Cursor;

use libfuzzer_sys::fuzz_target;
use tako_server::proxy_protocol::read_proxy_protocol;

fuzz_target!(|data: &[u8]| {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .expect("build runtime");
  runtime.block_on(async {
    let mut cursor = Cursor::new(data);
    // Result is intentionally ignored — we only care that the parser does
    // not panic, abort, or trip a sanitizer.
    let _ = read_proxy_protocol(&mut cursor).await;
  });
});
