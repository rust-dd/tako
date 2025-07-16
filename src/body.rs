//! HTTP request and response body handling utilities for efficient data processing.
//!
//! This module provides `TakoBody`, a flexible wrapper around HTTP body implementations
//! that supports various data sources including static content, streams, and dynamic
//! generation. It integrates with Hyper's body system while providing convenience methods
//! for common use cases like creating empty bodies, streaming data, and converting from
//! different input types with efficient memory management.
//!
//! # Examples
//!
//! ```rust
//! use tako::body::TakoBody;
//! use bytes::Bytes;
//! use futures_util::stream;
//!
//! // Create empty body
//! let empty = TakoBody::empty();
//!
//! // Create from string
//! let text_body = TakoBody::from("Hello, World!");
//!
//! // Create from bytes
//! let bytes_body = TakoBody::from(Bytes::from("Binary data"));
//!
//! // Create from stream
//! let stream_data = stream::iter(vec![
//!     Ok(Bytes::from("chunk1")),
//!     Ok(Bytes::from("chunk2")),
//! ]);
//! let stream_body = TakoBody::from_stream(stream_data);
//! ```

use std::{
    fmt::Debug,
    pin::Pin,
    task::{Context, Poll},
};

use bytes::Bytes;

use anyhow::Result;
use futures_util::{Stream, TryStream, TryStreamExt};
use http_body_util::{BodyExt, Empty, StreamBody};
use hyper::body::{Body, Frame, SizeHint};

use crate::types::{BoxBody, BoxError};

/// HTTP body wrapper with streaming and conversion support.
///
/// `TakoBody` provides a unified interface for handling HTTP request and response bodies
/// with support for various data sources. It wraps Hyper's body system with additional
/// convenience methods and efficient conversion capabilities. The implementation supports
/// both static content and streaming data while maintaining performance through zero-copy
/// operations where possible.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
/// use http_body_util::Full;
/// use bytes::Bytes;
///
/// // Static content
/// let static_body = TakoBody::from("Static response");
///
/// // Dynamic content
/// let dynamic = format!("User count: {}", 42);
/// let dynamic_body = TakoBody::from(dynamic);
///
/// // Binary data
/// let binary_data = vec![0u8, 1, 2, 3, 4];
/// let binary_body = TakoBody::from(binary_data);
///
/// // Empty response
/// let empty_body = TakoBody::empty();
/// ```
pub struct TakoBody(BoxBody);

impl TakoBody {
    /// Creates a new body from any type implementing the `Body` trait.
    ///
    /// This method wraps the provided body implementation with error mapping and
    /// boxing for type erasure. It enables storing different body types under
    /// a unified `TakoBody` interface while preserving streaming capabilities.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use http_body_util::Full;
    /// use bytes::Bytes;
    ///
    /// // Wrap a full body
    /// let content = Bytes::from("Hello, Tako!");
    /// let body = TakoBody::new(Full::from(content));
    ///
    /// // Wrap an empty body
    /// let empty = TakoBody::new(http_body_util::Empty::new());
    /// ```
    pub fn new<B>(body: B) -> Self
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        Self(body.map_err(|e| e.into()).boxed_unsync())
    }

    /// Creates a body from a stream of byte results.
    ///
    /// Converts a stream where each item is a `Result<Bytes, E>` into a streaming
    /// body. This is useful for handling data sources that may produce errors,
    /// such as file reading or network operations. Errors are automatically
    /// converted to the body's error type.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// // Create from successful chunks
    /// let chunks = vec![
    ///     Ok(Bytes::from("First chunk")),
    ///     Ok(Bytes::from("Second chunk")),
    ///     Ok(Bytes::from("Final chunk")),
    /// ];
    /// let stream = stream::iter(chunks);
    /// let body = TakoBody::from_stream(stream);
    ///
    /// // Handle potential errors in stream
    /// let error_stream = stream::iter(vec![
    ///     Ok(Bytes::from("Success")),
    ///     Err("Stream error"),
    /// ]);
    /// let error_body = TakoBody::from_stream(error_stream);
    /// ```
    pub fn from_stream<S, E>(stream: S) -> Self
    where
        S: Stream<Item = Result<Bytes, E>> + Send + 'static,
        E: Into<BoxError> + Debug + 'static,
    {
        let stream = stream.map_err(Into::into).map_ok(hyper::body::Frame::data);
        let body = StreamBody::new(stream).boxed_unsync();
        Self(body)
    }

    /// Creates a body from a stream of HTTP frames.
    ///
    /// Converts a `TryStream` of Hyper frames into a streaming body. This provides
    /// more control over the HTTP body format, allowing for metadata frames and
    /// advanced streaming patterns. The stream can include both data frames and
    /// trailers as defined by the HTTP specification.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use futures_util::stream;
    /// use hyper::body::Frame;
    /// use bytes::Bytes;
    ///
    /// // Create frame stream with data
    /// let frames = vec![
    ///     Ok(Frame::data(Bytes::from("Frame 1"))),
    ///     Ok(Frame::data(Bytes::from("Frame 2"))),
    /// ];
    /// let frame_stream = stream::iter(frames);
    /// let body = TakoBody::from_try_stream(frame_stream);
    /// ```
    pub fn from_try_stream<S, E>(stream: S) -> Self
    where
        S: TryStream<Ok = Frame<Bytes>, Error = E> + Send + 'static,
        E: Into<BoxError> + 'static,
    {
        let body = StreamBody::new(stream.map_err(Into::into)).boxed_unsync();
        Self(body)
    }

    /// Creates an empty body with no content.
    ///
    /// Returns a body that immediately signals end-of-stream without any data.
    /// This is useful for responses that only need status codes and headers
    /// without body content, such as 204 No Content or 304 Not Modified responses.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use hyper::body::Body;
    ///
    /// let empty = TakoBody::empty();
    /// assert!(empty.is_end_stream());
    /// assert_eq!(empty.size_hint().exact(), Some(0));
    /// ```
    pub fn empty() -> Self {
        Self::new(Empty::new())
    }
}

/// Provides a default empty body implementation.
///
/// The default implementation creates an empty body, which is useful for
/// initialization and cases where no content is needed.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
///
/// let default_body = TakoBody::default();
/// assert!(default_body.is_end_stream());
/// ```
impl Default for TakoBody {
    fn default() -> Self {
        Self::empty()
    }
}

/// Converts the unit type into an empty body.
///
/// This conversion allows using `()` as a body type in handlers, automatically
/// creating an empty response body. This is convenient for endpoints that only
/// need to return status codes without content.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
///
/// let unit_body = TakoBody::from(());
/// assert!(unit_body.is_end_stream());
/// ```
impl From<()> for TakoBody {
    fn from(_: ()) -> Self {
        Self::empty()
    }
}

/// Converts a string slice into a body with UTF-8 content.
///
/// Creates a body containing the string data as UTF-8 bytes. The string is
/// copied to create an owned version for the body content.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
/// use hyper::body::Body;
///
/// let body = TakoBody::from("Hello, World!");
/// assert!(!body.is_end_stream());
/// ```
impl From<&str> for TakoBody {
    fn from(buf: &str) -> Self {
        let owned = buf.to_owned();
        Self::new(http_body_util::Full::from(owned))
    }
}

/// Macro for implementing `From` conversions for various types.
///
/// This macro generates `From` implementations that convert the specified type
/// into a `TakoBody` using `http_body_util::Full` for efficient storage.
macro_rules! body_from_impl {
    ($ty:ty) => {
        impl From<$ty> for TakoBody {
            fn from(buf: $ty) -> Self {
                Self::new(http_body_util::Full::from(buf))
            }
        }
    };
}

/// Converts an owned string into a body with UTF-8 content.
///
/// Creates a body from the string's data without additional copying when
/// possible, making it efficient for dynamically generated content.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
///
/// let message = format!("Hello, user {}", 123);
/// let body = TakoBody::from(message);
/// ```
body_from_impl!(String);

/// Converts a byte vector into a body with binary content.
///
/// Creates a body from the vector's binary data, allowing for efficient
/// handling of non-text content such as images, files, or serialized data.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
///
/// let binary_data = vec![0x48, 0x65, 0x6c, 0x6c, 0x6f]; // "Hello" in bytes
/// let body = TakoBody::from(binary_data);
/// ```
body_from_impl!(Vec<u8>);

/// Converts `Bytes` into a body with efficient zero-copy handling.
///
/// Creates a body from `Bytes` data, which provides reference-counted byte
/// buffers for efficient memory usage and zero-copy operations when possible.
///
/// # Examples
///
/// ```rust
/// use tako::body::TakoBody;
/// use bytes::Bytes;
///
/// let data = Bytes::from_static(b"Static binary data");
/// let body = TakoBody::from(data);
/// ```
body_from_impl!(Bytes);

/// Implements the HTTP `Body` trait for streaming and polling operations.
///
/// This implementation enables `TakoBody` to be used as an HTTP body in Hyper
/// and other HTTP libraries. It delegates all operations to the inner boxed
/// body while providing the required type information and polling behavior.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::body::TakoBody;
/// use hyper::body::Body;
/// use std::pin::Pin;
/// use std::task::{Context, Poll};
///
/// async fn consume_body(mut body: TakoBody) {
///     // Body can be polled for frames
///     let size_hint = body.size_hint();
///     let is_empty = body.is_end_stream();
/// }
/// ```
impl Body for TakoBody {
    type Data = Bytes;
    type Error = BoxError;

    /// Polls for the next frame of body data.
    ///
    /// This method is called by the HTTP runtime to read body content in a
    /// streaming fashion. It delegates to the inner body implementation.
    #[inline]
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.0).poll_frame(cx)
    }

    /// Provides size hints for the body content.
    ///
    /// Returns information about the expected size of the body, which can be
    /// used for optimization and progress tracking.
    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }

    /// Indicates whether the body has reached the end of the stream.
    ///
    /// Returns `true` if no more data will be produced by this body, allowing
    /// for early termination and resource cleanup.
    #[inline]
    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }
}
