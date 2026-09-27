#![no_main]

use libfuzzer_sys::fuzz_target;
use tako_rs_streams::sse::SseEvent;

fuzz_target!(|data: &[u8]| {
  let Ok(text) = std::str::from_utf8(data) else {
    return;
  };
  let encoded = SseEvent::data(text).event(text).id(text).encode();
  assert!(encoded.ends_with(b"\n\n"));
  assert!(!encoded.contains(&b'\r'));
});
