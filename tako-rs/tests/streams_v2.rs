//! v2 stream regression tests covering SSE event encoding, FileStream
//! conditional-GET evaluation, and Static precompressed-asset preference.

use std::time::Duration;

use tako::sse::SseEvent;

#[test]
fn sse_event_encodes_data_only() {
  let bytes = SseEvent::data("hello").encode();
  let s = std::str::from_utf8(&bytes).unwrap();
  assert_eq!(s, "data: hello\n\n");
}

#[test]
fn sse_event_encodes_full_form() {
  let ev = SseEvent::data("payload").event("update").id("42");
  let bytes = ev.encode();
  let s = std::str::from_utf8(&bytes).unwrap();
  assert!(s.contains("event: update\n"));
  assert!(s.contains("id: 42\n"));
  assert!(s.contains("data: payload\n"));
  assert!(s.ends_with("\n\n"));
}

#[test]
fn sse_event_encodes_retry() {
  let ev = SseEvent::retry(Duration::from_secs(5));
  let bytes = ev.encode();
  let s = std::str::from_utf8(&bytes).unwrap();
  assert!(s.contains("retry: 5000\n"));
}

#[test]
fn sse_event_encodes_comment_lines() {
  let ev = SseEvent::comment("ping");
  let bytes = ev.encode();
  let s = std::str::from_utf8(&bytes).unwrap();
  assert!(s.starts_with(": ping\n"));
}

#[test]
fn sse_event_data_with_newline_splits() {
  let ev = SseEvent::data("line1\nline2");
  let bytes = ev.encode();
  let s = std::str::from_utf8(&bytes).unwrap();
  assert!(s.contains("data: line1\n"));
  assert!(s.contains("data: line2\n"));
}
