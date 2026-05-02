//! Server-Sent Events (SSE) implementation conforming to the W3C
//! [EventSource](https://html.spec.whatwg.org/multipage/server-sent-events.html)
//! specification.
//!
//! `Sse::new(stream)` keeps the original raw-bytes path (legacy: each `Bytes`
//! is wrapped as `data: …\n\n`). `Sse::events(stream)` is the v2 structured
//! API: each item is an [`SseEvent`] with `event:`, `id:`, `retry:`, and/or
//! comment fields. A configurable [`Sse::keep_alive`] periodically interleaves
//! comment frames so reverse proxies do not idle-close the connection.
//!
//! Additional defaults:
//! - `Cache-Control: no-cache, no-store, must-revalidate`
//! - `Connection: keep-alive`
//! - `X-Accel-Buffering: no` (defeats nginx response buffering)
//!
//! # Examples
//!
//! ```rust,ignore
//! use std::time::Duration;
//! use tako::sse::{Sse, SseEvent};
//! use tokio_stream::StreamExt as _;
//! use futures_util::stream;
//!
//! let events = stream::iter([
//!   SseEvent::data("hello"),
//!   SseEvent::data("again").event("greeting").id("1"),
//!   SseEvent::retry(Duration::from_secs(5)),
//! ]);
//!
//! Sse::events(events).keep_alive(Duration::from_secs(15));
//! ```

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
use tako_core::body::TakoBody;
use tako_core::responder::Responder;
use tako_core::types::Response;

const PREFIX: &[u8] = b"data: ";
const SUFFIX: &[u8] = b"\n\n";
const PS_LEN: usize = PREFIX.len() + SUFFIX.len();
const KEEPALIVE_FRAME: &[u8] = b":keepalive\n\n";

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
        buf.extend_from_slice(line.as_bytes());
        buf.extend_from_slice(b"\n");
      }
    }
    if let Some(e) = self.event.as_deref() {
      buf.extend_from_slice(b"event: ");
      buf.extend_from_slice(e.as_bytes());
      buf.extend_from_slice(b"\n");
    }
    if let Some(i) = self.id.as_deref() {
      buf.extend_from_slice(b"id: ");
      buf.extend_from_slice(i.as_bytes());
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
        buf.extend_from_slice(line.as_bytes());
        buf.extend_from_slice(b"\n");
      }
    }
    buf.extend_from_slice(b"\n");
    buf.freeze()
  }
}

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
/// reconnection ranges. Returns the trimmed header value when present.
pub fn last_event_id(headers: &http::HeaderMap) -> Option<String> {
  headers
    .get("last-event-id")
    .and_then(|v| v.to_str().ok())
    .map(|s| s.trim().to_string())
}
