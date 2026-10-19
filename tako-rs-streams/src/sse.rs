//! Server-Sent Events. Responses send `Cache-Control: no-cache, no-store, must-revalidate` and
//! `X-Accel-Buffering: no`, so neither caches nor nginx hold events back.
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
mod timer;

pub use event::SseEvent;
pub use stream::Sse;
pub use stream::SseEvents;
pub use stream::last_event_id;
pub use stream::last_event_id_bytes;
