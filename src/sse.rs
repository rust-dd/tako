/// This module provides the `Sse` struct, which is used to implement Server-Sent Events (SSE).
///
/// The `Sse` struct allows streaming data to clients in the SSE format, making it suitable for
/// real-time updates or push notifications over HTTP.
use std::convert::Infallible;

use bytes::{Bytes, BytesMut};
use http::{StatusCode, header};
use http_body_util::StreamBody;
use tokio_stream::{Stream, StreamExt};

use crate::{body::TakoBody, bytes::TakoBytes, responder::Responder, types::Response};

const PREFIX: &[u8] = b"data: ";
const SUFFIX: &[u8] = b"\n\n";

const fn ps_len() -> usize {
    PREFIX.len() + SUFFIX.len()
}

/// The `Sse` struct represents a Server-Sent Events (SSE) stream.
///
/// # Example
///
/// ```rust
/// use tako::sse::Sse;
/// use tako::bytes::TakoBytes;
/// use tokio_stream::StreamExt;
/// use tokio_stream::wrappers::IntervalStream;
/// use std::time::Duration;
/// use tokio::time::interval;
///
/// let stream = IntervalStream::new(interval(Duration::from_secs(1)))
///     .map(|_| TakoBytes::from("event data".to_string()));
///
/// let sse = Sse::new(stream);
/// // Use the `sse` instance as a responder to send events to the client.
/// ```
///
/// # Type Parameters
///
/// * `S` - A stream of `TakoBytes` items to be sent as SSE events.
pub struct Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    pub stream: S,
}

impl<S> Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    /// Creates a new `Sse` instance with the given stream.
    ///
    /// # Arguments
    ///
    /// * `stream` - A stream of `TakoBytes` items to be sent as SSE events.
    ///
    /// # Returns
    ///
    /// A new `Sse` instance.
    pub fn new(stream: S) -> Self {
        Self { stream }
    }
}

impl<S> Responder for Sse<S>
where
    S: Stream<Item = TakoBytes> + Send + 'static,
{
    /// Converts the `Sse` instance into an HTTP response.
    ///
    /// This method prepares the response with the appropriate headers and body
    /// to stream SSE events to the client.
    ///
    /// # Returns
    ///
    /// An HTTP response configured for SSE.
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
