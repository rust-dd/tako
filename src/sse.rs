//! Server-Sent Events (SSE) implementation for real-time data streaming.
//!
//! This module provides the `Sse` struct for implementing Server-Sent Events according to
//! the W3C EventSource specification. SSE enables servers to push data to web clients
//! over a single HTTP connection, making it ideal for real-time updates, live feeds,
//! and push notifications. The implementation handles proper SSE formatting with data
//! prefixes and event delimiters.
//!
//! # Examples
//!
//! ```rust
//! use tako::sse::Sse;
//! use bytes::Bytes;
//! use tokio_stream::{Stream, StreamExt};
//! use tokio_stream::wrappers::IntervalStream;
//! use std::time::Duration;
//! use tokio::time::interval;
//!
//! // Create a stream that emits events every second
//! let timer_stream = IntervalStream::new(interval(Duration::from_secs(1)))
//!     .map(|_| Bytes::from("Current time update".to_string()));
//!
//! let sse = Sse::new(timer_stream);
//! // Use as a responder in a route handler
//! ```

use std::convert::Infallible;

use bytes::{Bytes, BytesMut};
use http::{StatusCode, header};
use http_body_util::StreamBody;
use tokio_stream::{Stream, StreamExt};

use crate::{body::TakoBody, responder::Responder, types::Response};

const PREFIX: &[u8] = b"data: ";
const SUFFIX: &[u8] = b"\n\n";
const PS_LEN: usize = PREFIX.len() + SUFFIX.len();

/// Server-Sent Events stream wrapper for real-time data broadcasting.
///
/// `Sse` wraps a stream of `Bytes` and formats them according to the SSE
/// specification when converted to an HTTP response. It automatically handles
/// the required headers and event formatting, making it easy to implement
/// real-time features like live updates, notifications, or data feeds.
///
/// # Type Parameters
///
/// * `S` - Stream type that yields `Bytes` items for SSE events
///
/// # Examples
///
/// ```rust
/// use tako::sse::Sse;
/// use bytes::Bytes;
/// use tokio_stream::{StreamExt, iter};
///
/// // Create an SSE stream from a vector of messages
/// let messages = vec![
///     Bytes::from("First event".to_string()),
///     Bytes::from("Second event".to_string()),
///     Bytes::from("Third event".to_string()),
/// ];
///
/// let stream = iter(messages);
/// let sse = Sse::new(stream);
/// ```
pub struct Sse<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    /// The underlying stream of data to be sent as SSE events.
    pub stream: S,
}

impl<S> Sse<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    /// Creates a new SSE wrapper around the provided stream.
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> Responder for Sse<S>
where
    S: Stream<Item = Bytes> + Send + 'static,
{
    /// Converts the SSE stream into an HTTP response with proper headers.
    fn into_response(self) -> Response {
        let stream = self.stream.map(|msg| {
            let mut buf = BytesMut::with_capacity(PS_LEN + msg.len());
            buf.extend_from_slice(PREFIX);
            buf.extend_from_slice(&msg);
            buf.extend_from_slice(SUFFIX);
            Ok::<_, Infallible>(hyper::body::Frame::data(Bytes::from(buf)))
        });

        hyper::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/event-stream")
            .header(header::CACHE_CONTROL, "no-cache")
            .header(header::CONNECTION, "keep-alive")
            .body(TakoBody::new(StreamBody::new(stream)))
            .unwrap()
    }
}
