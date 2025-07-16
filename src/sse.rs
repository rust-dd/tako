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
//! use tako::bytes::TakoBytes;
//! use tokio_stream::{Stream, StreamExt};
//! use tokio_stream::wrappers::IntervalStream;
//! use std::time::Duration;
//! use tokio::time::interval;
//!
//! // Create a stream that emits events every second
//! let timer_stream = IntervalStream::new(interval(Duration::from_secs(1)))
//!     .map(|_| TakoBytes::from("Current time update".to_string()));
//!
//! let sse = Sse::new(timer_stream);
//! // Use as a responder in a route handler
//! ```

use std::convert::Infallible;

use bytes::{Bytes, BytesMut};
use http::{StatusCode, header};
use http_body_util::StreamBody;
use tokio_stream::{Stream, StreamExt};

use crate::{body::TakoBody, bytes::TakoBytes, responder::Responder, types::Response};

/// SSE data line prefix according to the EventSource specification.
///
/// Every SSE data line must start with "data: " followed by the actual content.
/// This constant ensures consistent formatting across all SSE messages.
const PREFIX: &[u8] = b"data: ";

/// SSE event terminator sequence.
///
/// Each SSE event must end with two newline characters ("\n\n") to signal
/// the end of the event to the client's EventSource parser.
const SUFFIX: &[u8] = b"\n\n";

/// Calculates the total length of SSE prefix and suffix bytes.
///
/// This const function computes the combined length of the SSE data prefix
/// and suffix for efficient buffer allocation during event formatting.
///
/// # Examples
///
/// ```rust
/// # const PREFIX: &[u8] = b"data: ";
/// # const SUFFIX: &[u8] = b"\n\n";
/// # const fn ps_len() -> usize {
/// #     PREFIX.len() + SUFFIX.len()
/// # }
/// assert_eq!(ps_len(), 8); // "data: " (6) + "\n\n" (2)
/// ```
const fn ps_len() -> usize {
    PREFIX.len() + SUFFIX.len()
}

/// Server-Sent Events stream wrapper for real-time data broadcasting.
///
/// `Sse` wraps a stream of `TakoBytes` and formats them according to the SSE
/// specification when converted to an HTTP response. It automatically handles
/// the required headers and event formatting, making it easy to implement
/// real-time features like live updates, notifications, or data feeds.
///
/// # Type Parameters
///
/// * `S` - Stream type that yields `TakoBytes` items for SSE events
///
/// # Examples
///
/// ```rust
/// use tako::sse::Sse;
/// use tako::bytes::TakoBytes;
/// use tokio_stream::{StreamExt, iter};
///
/// // Create an SSE stream from a vector of messages
/// let messages = vec![
///     TakoBytes::from("First event".to_string()),
///     TakoBytes::from("Second event".to_string()),
///     TakoBytes::from("Third event".to_string()),
/// ];
///
/// let stream = iter(messages);
/// let sse = Sse::new(stream);
/// ```
pub struct Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    /// The underlying stream of data to be sent as SSE events.
    pub stream: S,
}

impl<S> Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    /// Creates a new SSE wrapper around the provided stream.
    ///
    /// The stream should yield `TakoBytes` items that will be formatted as SSE
    /// events. Each item becomes a separate SSE event sent to connected clients.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::sse::Sse;
    /// use tako::bytes::TakoBytes;
    /// use tokio_stream::{StreamExt, wrappers::IntervalStream};
    /// use std::time::Duration;
    /// use tokio::time::interval;
    ///
    /// // Create a periodic update stream
    /// let updates = IntervalStream::new(interval(Duration::from_millis(500)))
    ///     .enumerate()
    ///     .map(|(i, _)| TakoBytes::from(format!("Update #{}", i)));
    ///
    /// let sse = Sse::new(updates);
    /// ```
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> Responder for Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    /// Converts the SSE stream into an HTTP response with proper headers.
    ///
    /// This method configures the response with the required SSE headers including
    /// Content-Type, Cache-Control, and Connection headers. It also formats each
    /// stream item with the proper SSE data prefix and event terminator.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::sse::Sse;
    /// use tako::responder::Responder;
    /// use tako::bytes::TakoBytes;
    /// use tokio_stream::iter;
    /// use http::StatusCode;
    ///
    /// let messages = vec![TakoBytes::from("Hello, SSE!".to_string())];
    /// let sse = Sse::new(iter(messages));
    /// let response = sse.into_response();
    ///
    /// assert_eq!(response.status(), StatusCode::OK);
    /// assert_eq!(
    ///     response.headers().get("content-type").unwrap(),
    ///     "text/event-stream"
    /// );
    /// assert_eq!(
    ///     response.headers().get("cache-control").unwrap(),
    ///     "no-cache"
    /// );
    /// ```
    fn into_response(self) -> Response {
        let stream = self.stream.map(|TakoBytes(msg)| {
            let mut buf = BytesMut::with_capacity(ps_len() + msg.len());
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
