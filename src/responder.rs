/// This module provides the `Responder` trait, which defines a common interface for converting various types into HTTP responses.
/// It includes implementations for common types like `String`, `&'static str`, and `()`.
use std::convert::Infallible;

use bytes::Bytes;
use http::{HeaderName, HeaderValue, Response, StatusCode};
use http_body_util::Full;

use crate::body::TakoBody;

/// The `Responder` trait defines a method for converting a type into an HTTP response.
///
/// Types implementing this trait can be used as return values in request handlers,
/// allowing seamless conversion into `Response<TakoBody>`.
///
/// # Example
///
/// ```rust
/// use tako::responder::Responder;
/// use tako::body::TakoBody;
/// use http::Response;
///
/// impl Responder for &'static str {
///     fn into_response(self) -> Response<TakoBody> {
///         Response::new(TakoBody::from(self))
///     }
/// }
/// ```
pub trait Responder {
    fn into_response(self) -> Response<TakoBody>;
}

/// Implementation of the `Responder` trait for `Response<TakoBody>`.
///
/// This allows an existing `Response<TakoBody>` to be directly used as a response
/// without any additional conversion.
impl Responder for Response<TakoBody> {
    fn into_response(self) -> Response<TakoBody> {
        self
    }
}

/// Implementation of the `Responder` trait for `&'static str`.
///
/// This converts a static string slice into an HTTP response with a `text/plain` body.
impl Responder for &'static str {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from_static(
            self.as_bytes(),
        ))))
    }
}

/// Implementation of the `Responder` trait for `String`.
///
/// This converts a `String` into an HTTP response with a `text/plain` body.
impl Responder for String {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from(self))))
    }
}

/// Implementation of the `Responder` trait for `()`.
///
/// This creates an empty HTTP response with no body.
impl Responder for () {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::empty())
    }
}

/// Implementation of the `Responder` trait for `Infallible`.
///
/// Since `Infallible` cannot have any value, this implementation is unreachable.
impl Responder for Infallible {
    fn into_response(self) -> Response<TakoBody> {
        match self {}
    }
}

impl<const N: usize> Responder for (StatusCode, [(HeaderName, &'static str); N]) {
    fn into_response(self) -> Response<TakoBody> {
        let (status, headers) = self;

        let mut res = Response::new(TakoBody::empty());
        *res.status_mut() = status;

        for (name, value) in headers {
            res.headers_mut()
                .append(name, HeaderValue::from_static(value));
        }
        res
    }
}
