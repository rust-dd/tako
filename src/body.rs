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
  pub fn new<B>(body: B) -> Self
  where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
  {
    Self(body.map_err(|e| e.into()).boxed_unsync())
  }

  /// Creates a body from a stream of byte results.
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
  pub fn from_try_stream<S, E>(stream: S) -> Self
  where
    S: TryStream<Ok = Frame<Bytes>, Error = E> + Send + 'static,
    E: Into<BoxError> + 'static,
  {
    let body = StreamBody::new(stream.map_err(Into::into)).boxed_unsync();
    Self(body)
  }

  /// Creates an empty body with no content.
  pub fn empty() -> Self {
    Self::new(Empty::new())
  }
}

/// Provides a default empty body implementation.
impl Default for TakoBody {
  fn default() -> Self {
    Self::empty()
  }
}

impl From<()> for TakoBody {
  fn from(_: ()) -> Self {
    Self::empty()
  }
}

impl From<&str> for TakoBody {
  fn from(buf: &str) -> Self {
    let owned = buf.to_owned();
    Self::new(http_body_util::Full::from(owned))
  }
}

/// Macro for implementing `From` conversions for various types.
macro_rules! body_from_impl {
  ($ty:ty) => {
    impl From<$ty> for TakoBody {
      fn from(buf: $ty) -> Self {
        Self::new(http_body_util::Full::from(buf))
      }
    }
  };
}

body_from_impl!(String);
body_from_impl!(Vec<u8>);
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
  #[inline]
  fn poll_frame(
    mut self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
    Pin::new(&mut self.0).poll_frame(cx)
  }

  /// Provides size hints for the body content.
  #[inline]
  fn size_hint(&self) -> SizeHint {
    self.0.size_hint()
  }

  /// Indicates whether the body has reached the end of the stream.
  #[inline]
  fn is_end_stream(&self) -> bool {
    self.0.is_end_stream()
  }
}
