/// This module provides the `TakoBody` struct, which is a wrapper around a boxed HTTP body.
/// It includes utility methods for creating and manipulating HTTP bodies, as well as
/// implementations for common traits like `Default` and `Body`.
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

/// The `TakoBody` struct is a wrapper around a boxed HTTP body (`BoxedBody`).
/// It provides utility methods for creating empty bodies and converting various types
/// into HTTP bodies.
///
/// # Example
///
/// ```rust
/// use tako::body::TakoBody;
/// use http_body_util::Empty;
///
/// let empty_body = TakoBody::empty();
/// let string_body = TakoBody::from("Hello, world!".to_string());
/// ```
pub struct TakoBody(BoxBody);

impl TakoBody {
    /// Creates a new `TakoBody` from a given body.
    ///
    /// # Arguments
    ///
    /// * `body` - The body to wrap, which must implement the `Body` trait.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use http_body_util::Full;
    /// use bytes::Bytes;
    ///
    /// let body = TakoBody::new(Full::from(Bytes::from("Hello")));
    /// ```
    pub fn new<B>(body: B) -> Self
    where
        B: Body<Data = Bytes> + Send + 'static,
        B::Error: Into<BoxError>,
    {
        Self(body.map_err(|e| e.into()).boxed_unsync())
    }

    /// Creates a `TakoBody` from a stream of `Result<Bytes, E>`.
    ///
    /// # Arguments
    ///
    /// * `stream` - A stream where each item is a `Result` containing either `Bytes` or an error.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use futures_util::stream;
    /// use bytes::Bytes;
    ///
    /// let stream = stream::iter(vec![Ok(Bytes::from("Hello")), Ok(Bytes::from("World"))]);
    /// let body = TakoBody::from_stream(stream);
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

    /// Creates a `TakoBody` from a `TryStream` of `hyper::body::Frame<Bytes>`.
    ///
    /// # Arguments
    ///
    /// * `stream` - A `TryStream` where each item is a `hyper::body::Frame` containing `Bytes`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    /// use futures_util::stream;
    /// use hyper::body::Frame;
    /// use bytes::Bytes;
    ///
    /// let stream = stream::iter(vec![Ok(Frame::data(Bytes::from("Hello"))), Ok(Frame::data(Bytes::from("World")))]);
    /// let body = TakoBody::from_try_stream(stream);
    /// ```
    pub fn from_try_stream<S, E>(stream: S) -> Self
    where
        S: TryStream<Ok = Frame<Bytes>, Error = E> + Send + 'static,
        E: Into<BoxError> + 'static,
    {
        let body = StreamBody::new(stream.map_err(Into::into)).boxed_unsync();
        Self(body)
    }

    /// Creates an empty `TakoBody`.
    ///
    /// # Example
    ///
    /// ```rust
    /// use tako::body::TakoBody;
    ///
    /// let empty_body = TakoBody::empty();
    /// ```
    pub fn empty() -> Self {
        Self::new(Empty::new())
    }
}

/// Provides a default implementation for `TakoBody`, which returns an empty body.
impl Default for TakoBody {
    fn default() -> Self {
        Self::empty()
    }
}

/// Implements conversion from `()` to `TakoBody`, resulting in an empty body.
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

/// Implements the `Body` trait for `TakoBody`, allowing it to be used as an HTTP body.
///
/// This implementation delegates the actual body operations to the inner `BoxedBody`.
impl Body for TakoBody {
    type Data = Bytes;
    type Error = BoxError;

    #[inline]
    fn poll_frame(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Pin::new(&mut self.0).poll_frame(cx)
    }

    #[inline]
    fn size_hint(&self) -> SizeHint {
        self.0.size_hint()
    }

    #[inline]
    fn is_end_stream(&self) -> bool {
        self.0.is_end_stream()
    }
}
