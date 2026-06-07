//! Server-Sent Events (SSE) implementation conforming to the W3C
//! [EventSource](https://html.spec.whatwg.org/multipage/server-sent-events.html)
//! specification.
//!
//! `Sse::new(stream)` keeps the original raw-bytes path (legacy: each `Bytes`
//! is wrapped as `data: …\n\n`). `Sse::events(stream)` is the v2 structured
//! API: each item is an [`SseEvent`](crate::sse::SseEvent) with `event:`, `id:`, `retry:`, and/or
//! comment fields. A configurable [`Sse::keep_alive`](crate::sse::Sse::keep_alive) periodically interleaves
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

mod event;
mod stream;

pub use event::SseEvent;
pub use stream::Sse;
pub use stream::SseEvents;
pub use stream::last_event_id;
pub use stream::last_event_id_bytes;
