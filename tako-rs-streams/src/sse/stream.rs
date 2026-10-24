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
use super::timer::KeepAliveTimer;

const PREFIX: &[u8] = b"data: ";
const SUFFIX: &[u8] = b"\n\n";
const PS_LEN: usize = PREFIX.len() + SUFFIX.len();
const KEEPALIVE_FRAME: &[u8] = b":keepalive\n\n";

/// An SSE responder over raw payloads; [`Sse::events`] builds one over structured events.
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
  /// Sends each item as `data: <item>\n\n` unescaped, so a newline in untrusted bytes can inject
  /// events; use [`Sse::events`] for those.
  pub fn new(stream: S) -> Self {
    Self {
      stream,
      keepalive: None,
    }
  }
}

impl<S> Sse<S> {
  /// Sends a `:keepalive` comment whenever `period` passes without an event.
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

/// The responder [`Sse::events`] builds.
pub struct SseEvents<S> {
  stream: S,
  keepalive: Option<Duration>,
}

impl<S> Sse<S> {
  /// A responder whose events are encoded with [`SseEvent::encode`].
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
  /// Sends a `:keepalive` comment whenever `period` passes without an event.
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
    .header("X-Accel-Buffering", "no")
    .body(body)
    .expect("valid SSE response")
}

pin_project! {
  /// The keepalive timer restarts whenever the inner stream yields.
  struct KeepAliveStream<S> {
    #[pin]
    inner: S,
    timer: KeepAliveTimer,
    period: Duration,
    keepalive_frame: Bytes,
    inner_done: bool,
  }
}

impl<S> KeepAliveStream<S> {
  fn new(inner: S, period: Duration, keepalive_frame: Bytes) -> Self {
    Self {
      inner,
      timer: KeepAliveTimer::new(period),
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
          this.timer.reset(*this.period);
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

    if this.timer.poll_elapsed(cx).is_ready() {
      let frame = http_body::Frame::data(this.keepalive_frame.clone());
      this.timer.reset(*this.period);
      return Poll::Ready(Some(Ok(frame)));
    }

    Poll::Pending
  }
}

/// The trimmed `Last-Event-ID` of a reconnecting client; `None` unless it is visible ASCII, which
/// [`last_event_id_bytes`] does not require.
pub fn last_event_id(headers: &http::HeaderMap) -> Option<String> {
  headers
    .get("last-event-id")
    .and_then(|v| v.to_str().ok())
    .map(|s| s.trim().to_string())
}

/// [`last_event_id`] as raw bytes, trimmed of ASCII whitespace.
pub fn last_event_id_bytes(headers: &http::HeaderMap) -> Option<Vec<u8>> {
  let bytes = headers.get("last-event-id")?.as_bytes();
  let start = bytes.iter().position(|b| !b.is_ascii_whitespace())?;
  let end = bytes
    .iter()
    .rposition(|b| !b.is_ascii_whitespace())
    .map_or(start, |i| i + 1);
  Some(bytes[start..end].to_vec())
}

#[cfg(test)]
mod tests;
