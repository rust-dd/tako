use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;

/// A single SSE event.
///
/// Build with [`SseEvent::data`] / [`SseEvent::comment`] / [`SseEvent::retry`]
/// then chain [`SseEvent::event`] / [`SseEvent::id`].
#[derive(Debug, Clone, Default)]
pub struct SseEvent {
  /// `data:` payload — multi-line strings are split into multiple `data:` fields.
  pub data: Option<String>,
  /// `event:` field — the event name handler.
  pub event: Option<String>,
  /// `id:` field — sets the `Last-Event-ID` for client reconnection.
  pub id: Option<String>,
  /// `retry:` field — reconnection delay hint, in milliseconds.
  pub retry_ms: Option<u64>,
  /// `:` comment — invisible to handlers, useful for keepalive.
  pub comment: Option<String>,
}

impl SseEvent {
  /// New event carrying a single `data:` payload.
  pub fn data(d: impl Into<String>) -> Self {
    Self {
      data: Some(d.into()),
      ..Default::default()
    }
  }

  /// New event consisting only of a comment line.
  pub fn comment(c: impl Into<String>) -> Self {
    Self {
      comment: Some(c.into()),
      ..Default::default()
    }
  }

  /// New event setting the reconnection retry hint.
  pub fn retry(d: Duration) -> Self {
    Self {
      retry_ms: Some(d.as_millis() as u64),
      ..Default::default()
    }
  }

  /// Set the `event:` field.
  pub fn event(mut self, e: impl Into<String>) -> Self {
    self.event = Some(e.into());
    self
  }

  /// Set the `id:` field.
  pub fn id(mut self, i: impl Into<String>) -> Self {
    self.id = Some(i.into());
    self
  }

  /// Encode as a single SSE wire frame.
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
      // `event` is a single-line field per SSE spec; collapse any embedded
      // CR/LF the caller may have included to keep an attacker from
      // injecting synthetic fields (`event: foo\nid: hostile`).
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

/// Replace SSE-control characters with a space so single-line fields cannot
/// smuggle extra `event:` / `id:` lines.
fn sanitize_single_line(s: &str) -> String {
  s.replace(['\n', '\r'], " ")
}

/// Strip lone `\r`s from a line value. (`\n` is already handled by the caller
/// which splits on it.)
fn strip_cr(s: &str) -> String {
  s.replace('\r', "")
}

#[cfg(test)]
mod tests {
  use super::SseEvent;

  #[test]
  fn event_and_id_strip_crlf() {
    // Attacker-controlled string tries to inject a fake `id:` line.
    let frame = SseEvent::data("payload")
      .event("legit\nid: hostile")
      .id("a\r\nb")
      .encode();
    let s = std::str::from_utf8(&frame).unwrap();
    // The `event:` line must collapse the embedded newline so no synthetic
    // SSE field appears (each \n/\r → single space).
    assert!(
      s.contains("event: legit id: hostile\n"),
      "expected sanitized event line, got: {s:?}"
    );
    assert!(
      s.contains("id: a  b\n"),
      "expected sanitized id line, got: {s:?}"
    );
    // No raw control characters anywhere inside the value bytes.
    assert!(!s.contains('\r'));
  }
}
