use std::convert::Infallible;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;
use std::time::Duration;

use bytes::Bytes;
use bytes::BytesMut;
use futures_util::Stream;
use futures_util::StreamExt;
use http::StatusCode;
use http::header;
use http_body_util::StreamBody;
use pin_project_lite::pin_project;
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

use super::SseEvent;

const PREFIX: &[u8] = b"data: ";
const SUFFIX: &[u8] = b"\n\n";
const PS_LEN: usize = PREFIX.len() + SUFFIX.len();
const KEEPALIVE_FRAME: &[u8] = b":keepalive\n\n";

/// Server-Sent Events stream wrapper for real-time data broadcasting.
#[doc(alias = "sse")]
#[doc(alias = "eventsource")]
pub struct Sse<S> {
  pub(crate) stream: S,
  pub(crate) keepalive: Option<Duration>,
}

impl<S> Sse<S>
where
  S: Stream<Item = Bytes> + Send + 'static,
{
  /// Legacy constructor — wraps each `Bytes` as `data: …\n\n` (W3C minimum).
  ///
  /// **⚠️ Security note (STR-5):** this raw-bytes wrapper does NOT sanitize
  /// embedded `\n` / `\r` / `\r\n` sequences. A message containing
  /// `\n\nevent:click\n\n` is interpreted by the browser as **two separate
  /// SSE events** — a synthetic event/field injection if the message comes
  /// from untrusted input. The structured [`Sse::events`] path is safe
  /// (every line is rebuilt with strict `data:`/`event:` prefixes and CR is
  /// stripped). Use `Sse::events` for any message that could carry
  /// caller-controlled bytes; reserve `Sse::new` for already-encoded raw
  /// SSE chunks the caller produced.
  ///
  /// For richer events (`event:`, `id:`, `retry:`, comments) use
  /// [`Sse::events`] which accepts a stream of [`SseEvent`].
  pub fn new(stream: S) -> Self {
    Self {
      stream,
      keepalive: None,
    }
  }
}

impl<S> Sse<S> {
  /// Periodically interleave `:keepalive\n\n` comment frames into the stream.
  pub fn keep_alive(mut self, period: Duration) -> Self {
    self.keepalive = Some(period);
    self
  }
}

impl<S> Responder for Sse<S>
where
  S: Stream<Item = Bytes> + Send + 'static,
{
  fn into_response(self) -> Response {
    let mapped = self.stream.map(|msg| {
      let mut buf = BytesMut::with_capacity(PS_LEN + msg.len());
      buf.extend_from_slice(PREFIX);
      buf.extend_from_slice(&msg);
      buf.extend_from_slice(SUFFIX);
      Ok::<_, Infallible>(http_body::Frame::data(Bytes::from(buf)))
    });

    let body = if let Some(period) = self.keepalive {
      let stream = KeepAliveStream::new(mapped, period, Bytes::from_static(KEEPALIVE_FRAME));
      TakoBody::new(StreamBody::new(stream))
    } else {
      TakoBody::new(StreamBody::new(mapped))
    };

    build_sse_response(body)
  }
}

/// Structured SSE responder — accepts a stream of [`SseEvent`].
pub struct SseEvents<S> {
  stream: S,
  keepalive: Option<Duration>,
}

impl<S> Sse<S> {
  /// Build a structured SSE responder from a stream of [`SseEvent`].
  pub fn events(stream: S) -> SseEvents<S>
  where
    S: Stream<Item = SseEvent> + Send + 'static,
  {
    SseEvents {
      stream,
      keepalive: None,
    }
  }
}

impl<S> SseEvents<S> {
  /// Periodically interleave `:keepalive\n\n` comment frames into the stream.
  pub fn keep_alive(mut self, period: Duration) -> Self {
    self.keepalive = Some(period);
    self
  }
}

impl<S> Responder for SseEvents<S>
where
  S: Stream<Item = SseEvent> + Send + 'static,
{
  fn into_response(self) -> Response {
    let mapped = self
      .stream
      .map(|ev| Ok::<_, Infallible>(http_body::Frame::data(ev.encode())));

    let body = if let Some(period) = self.keepalive {
      let stream = KeepAliveStream::new(mapped, period, Bytes::from_static(KEEPALIVE_FRAME));
      TakoBody::new(StreamBody::new(stream))
    } else {
      TakoBody::new(StreamBody::new(mapped))
    };

    build_sse_response(body)
  }
}

fn build_sse_response(body: TakoBody) -> Response {
  http::Response::builder()
    .status(StatusCode::OK)
    .header(header::CONTENT_TYPE, "text/event-stream")
    .header(header::CACHE_CONTROL, "no-cache, no-store, must-revalidate")
    .header(header::CONNECTION, "keep-alive")
    .header("X-Accel-Buffering", "no")
    .body(body)
    .expect("valid SSE response")
}

pin_project! {
  /// Wraps an inner SSE-frame stream, interleaving `:keepalive\n\n` comments
  /// every `period` interval. The keepalive timer resets whenever the inner
  /// stream produces an item.
  struct KeepAliveStream<S> {
    #[pin]
    inner: S,
    #[pin]
    sleep: tokio::time::Sleep,
    period: Duration,
    keepalive_frame: Bytes,
    inner_done: bool,
  }
}

impl<S> KeepAliveStream<S> {
  fn new(inner: S, period: Duration, keepalive_frame: Bytes) -> Self {
    Self {
      inner,
      sleep: tokio::time::sleep(period),
      period,
      keepalive_frame,
      inner_done: false,
    }
  }
}

impl<S> Stream for KeepAliveStream<S>
where
  S: Stream<Item = Result<http_body::Frame<Bytes>, Infallible>>,
{
  type Item = Result<http_body::Frame<Bytes>, Infallible>;

  fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
    let mut this = self.project();
    if !*this.inner_done {
      match this.inner.as_mut().poll_next(cx) {
        Poll::Ready(Some(item)) => {
          let deadline = tokio::time::Instant::now() + *this.period;
          this.sleep.as_mut().reset(deadline);
          return Poll::Ready(Some(item));
        }
        Poll::Ready(None) => {
          *this.inner_done = true;
        }
        Poll::Pending => {}
      }
    }

    if *this.inner_done {
      return Poll::Ready(None);
    }

    if this.sleep.as_mut().poll(cx).is_ready() {
      let frame = http_body::Frame::data(this.keepalive_frame.clone());
      let deadline = tokio::time::Instant::now() + *this.period;
      this.sleep.as_mut().reset(deadline);
      return Poll::Ready(Some(Ok(frame)));
    }

    Poll::Pending
  }
}

/// `Last-Event-ID` request header helper.
///
/// Handlers building an SSE stream can call this to honor client-side
/// reconnection ranges. Returns the trimmed header value when present and
/// well-formed UTF-8. Use [`last_event_id_bytes`] when the application
/// emits non-UTF-8 ids (e.g. binary cursors) so they round-trip through
/// reconnects intact.
pub fn last_event_id(headers: &http::HeaderMap) -> Option<String> {
  headers
    .get("last-event-id")
    .and_then(|v| v.to_str().ok())
    .map(|s| s.trim().to_string())
}

/// Byte-preserving variant of [`last_event_id`].
///
/// Returns the raw header bytes (with surrounding ASCII whitespace trimmed)
/// regardless of UTF-8 validity. Useful when the server emits opaque binary
/// cursors as `id:` values — the UTF-8-only helper silently drops those on
/// reconnect, breaking event continuity.
pub fn last_event_id_bytes(headers: &http::HeaderMap) -> Option<Vec<u8>> {
  let bytes = headers.get("last-event-id")?.as_bytes();
  let start = bytes.iter().position(|b| !b.is_ascii_whitespace())?;
  let end = bytes
    .iter()
    .rposition(|b| !b.is_ascii_whitespace())
    .map_or(start, |i| i + 1);
  Some(bytes[start..end].to_vec())
}
