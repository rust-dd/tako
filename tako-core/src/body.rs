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

use std::convert::Infallible;
use std::fmt::Debug;
use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use anyhow::Result;
use bytes::Bytes;
use futures_util::Stream;
use futures_util::TryStream;
use futures_util::TryStreamExt;
use http_body::Body;
use http_body::Frame;
use http_body::SizeHint;
use http_body_util::BodyExt;
use http_body_util::Empty;
use http_body_util::Full;
use http_body_util::StreamBody;

use crate::types::BoxBody;
use crate::types::BoxError;

/// Internal enum to avoid heap-boxing for the most common body kinds.
/// `Full`, `Empty`, and `Incoming` are stored inline (zero allocations).
/// Anything else (streams, mapped bodies, etc.) goes through the `Boxed` variant.
#[allow(dead_code)]
enum BodyInner {
  Full(Full<Bytes>),
  Empty(Empty<Bytes>),
  /// Hyper's incoming request body — stored inline to avoid boxing on every request.
  Incoming(hyper::body::Incoming),
  Boxed(BoxBody),
}

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
pub struct TakoBody(BodyInner);

impl std::fmt::Debug for TakoBody {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("TakoBody").finish_non_exhaustive()
  }
}

impl TakoBody {
  /// Creates a new body from any type implementing the `Body` trait.
  ///
  /// This is the generic (boxing) path — prefer [`full`](Self::full) or
  /// [`empty`](Self::empty) when the concrete type is known.
  #[inline]
  pub fn new<B>(body: B) -> Self
  where
    B: Body<Data = Bytes> + Send + 'static,
    B::Error: Into<BoxError>,
  {
    Self(BodyInner::Boxed(body.map_err(|e| e.into()).boxed_unsync()))
  }

  /// Creates a body from a `Full<Bytes>` **without heap-boxing**.
  #[inline]
  pub fn full(body: Full<Bytes>) -> Self {
    Self(BodyInner::Full(body))
  }

  /// Wraps a `hyper::body::Incoming` **without heap-boxing**.
  #[inline]
  #[doc(hidden)]
  pub fn incoming(body: hyper::body::Incoming) -> Self {
    Self(BodyInner::Incoming(body))
  }

  /// Creates a body from a stream of byte results.
  #[inline]
  pub fn from_stream<S, E>(stream: S) -> Self
  where
    S: Stream<Item = Result<Bytes, E>> + Send + 'static,
    E: Into<BoxError> + Debug + 'static,
  {
    let stream = stream.map_err(Into::into).map_ok(http_body::Frame::data);
    let body = StreamBody::new(stream).boxed_unsync();
    Self(BodyInner::Boxed(body))
  }

  /// Creates a body from a stream of HTTP frames.
  #[inline]
  pub fn from_try_stream<S, E>(stream: S) -> Self
  where
    S: TryStream<Ok = Frame<Bytes>, Error = E> + Send + 'static,
    E: Into<BoxError> + 'static,
  {
    let body = StreamBody::new(stream.map_err(Into::into)).boxed_unsync();
    Self(BodyInner::Boxed(body))
  }

  /// Creates an empty body with no content **without heap-boxing**.
  #[inline]
  #[must_use]
  pub fn empty() -> Self {
    Self(BodyInner::Empty(Empty::new()))
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
    Self::full(Full::from(Bytes::from(buf.to_owned())))
  }
}

impl From<String> for TakoBody {
  fn from(buf: String) -> Self {
    Self::full(Full::from(Bytes::from(buf)))
  }
}

impl From<Vec<u8>> for TakoBody {
  fn from(buf: Vec<u8>) -> Self {
    Self::full(Full::from(Bytes::from(buf)))
  }
}

impl From<Bytes> for TakoBody {
  fn from(buf: Bytes) -> Self {
    Self::full(Full::from(buf))
  }
}

/// Converts an `Infallible` poll result into a `BoxError` poll result at zero cost.
#[inline]
fn map_infallible_frame(
  poll: Poll<Option<core::result::Result<Frame<Bytes>, Infallible>>>,
) -> Poll<Option<core::result::Result<Frame<Bytes>, BoxError>>> {
  poll.map(|opt| opt.map(|res| res.map_err(|e| match e {})))
}

/// Converts a `hyper::Error` poll result into a `BoxError` poll result.
#[inline]
fn map_hyper_frame(
  poll: Poll<Option<core::result::Result<Frame<Bytes>, hyper::Error>>>,
) -> Poll<Option<core::result::Result<Frame<Bytes>, BoxError>>> {
  poll.map(|opt| opt.map(|res| res.map_err(Into::into)))
}

impl Body for TakoBody {
  type Data = Bytes;
  type Error = BoxError;

  #[inline]
  fn poll_frame(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
  ) -> Poll<Option<core::result::Result<Frame<Self::Data>, Self::Error>>> {
    // All variants are Unpin, so get_mut is safe.
    match &mut self.get_mut().0 {
      BodyInner::Full(body) => map_infallible_frame(Pin::new(body).poll_frame(cx)),
      BodyInner::Empty(body) => map_infallible_frame(Pin::new(body).poll_frame(cx)),
      BodyInner::Incoming(body) => map_hyper_frame(Pin::new(body).poll_frame(cx)),
      BodyInner::Boxed(body) => Pin::new(body).poll_frame(cx),
    }
  }

  #[inline]
  fn size_hint(&self) -> SizeHint {
    match &self.0 {
      BodyInner::Full(body) => body.size_hint(),
      BodyInner::Empty(body) => body.size_hint(),
      BodyInner::Incoming(body) => body.size_hint(),
      BodyInner::Boxed(body) => body.size_hint(),
    }
  }

  #[inline]
  fn is_end_stream(&self) -> bool {
    match &self.0 {
      BodyInner::Full(body) => body.is_end_stream(),
      BodyInner::Empty(body) => body.is_end_stream(),
      BodyInner::Incoming(body) => body.is_end_stream(),
      BodyInner::Boxed(body) => body.is_end_stream(),
    }
  }
}
