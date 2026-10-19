use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;

/// One event frame; newlines in `event` and `id` become spaces, while `data` and `comment`
/// span one field per line.
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
  pub data: Option<String>,
  pub event: Option<String>,
  /// Sent back by the client as `Last-Event-ID` when it reconnects.
  pub id: Option<String>,
  /// Reconnection delay in milliseconds.
  pub retry_ms: Option<u64>,
  /// Ignored by `EventSource`, which makes it a keepalive.
  pub comment: Option<String>,
}

impl SseEvent {
  pub fn data(d: impl Into<String>) -> Self {
    Self {
      data: Some(d.into()),
      ..Default::default()
    }
  }

  pub fn comment(c: impl Into<String>) -> Self {
    Self {
      comment: Some(c.into()),
      ..Default::default()
    }
  }

  pub fn retry(d: Duration) -> Self {
    Self {
      retry_ms: Some(d.as_millis() as u64),
      ..Default::default()
    }
  }

  pub fn event(mut self, e: impl Into<String>) -> Self {
    self.event = Some(e.into());
    self
  }

  pub fn id(mut self, i: impl Into<String>) -> Self {
    self.id = Some(i.into());
    self
  }

  /// The wire frame, ending in a blank line.
  pub fn encode(&self) -> Bytes {
    let mut buf = BytesMut::with_capacity(64);
    if let Some(c) = self.comment.as_deref() {
      for line in c.split('\n') {
        buf.extend_from_slice(b": ");
        buf.extend_from_slice(strip_cr(line).as_bytes());
        buf.extend_from_slice(b"\n");
      }
    }
    if let Some(e) = self.event.as_deref() {
      buf.extend_from_slice(b"event: ");
      buf.extend_from_slice(sanitize_single_line(e).as_bytes());
      buf.extend_from_slice(b"\n");
    }
    if let Some(i) = self.id.as_deref() {
      buf.extend_from_slice(b"id: ");
      buf.extend_from_slice(sanitize_single_line(i).as_bytes());
      buf.extend_from_slice(b"\n");
    }
    if let Some(r) = self.retry_ms {
      buf.extend_from_slice(b"retry: ");
      buf.extend_from_slice(r.to_string().as_bytes());
      buf.extend_from_slice(b"\n");
    }
    if let Some(d) = self.data.as_deref() {
      for line in d.split('\n') {
        buf.extend_from_slice(b"data: ");
        buf.extend_from_slice(strip_cr(line).as_bytes());
        buf.extend_from_slice(b"\n");
      }
    }
    buf.extend_from_slice(b"\n");
    buf.freeze()
  }
}

/// CR and LF become spaces, so a single-line field cannot inject fields such as `id:`.
fn sanitize_single_line(s: &str) -> String {
  s.replace(['\n', '\r'], " ")
}

/// Callers split on `\n` first.
fn strip_cr(s: &str) -> String {
  s.replace('\r', "")
}

#[cfg(test)]
mod tests {
  use super::SseEvent;

  #[test]
  fn event_and_id_strip_crlf() {
    let frame = SseEvent::data("payload")
      .event("legit\nid: hostile")
      .id("a\r\nb")
      .encode();
    let s = std::str::from_utf8(&frame).unwrap();
    assert!(
      s.contains("event: legit id: hostile\n"),
      "expected sanitized event line, got: {s:?}"
    );
    assert!(
      s.contains("id: a  b\n"),
      "expected sanitized id line, got: {s:?}"
    );
    assert!(!s.contains('\r'));
  }
}
