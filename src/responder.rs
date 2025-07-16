//! Response generation utilities and trait implementations for HTTP responses.
//!
//! This module provides the core `Responder` trait that enables various types to be
//! converted into HTTP responses. It includes implementations for common types like
//! strings, status codes, and custom response types. The trait allows handlers to
//! return different types that are automatically converted to proper HTTP responses.
//!
//! # Examples
//!
//! ```rust
//! use tako::responder::Responder;
//! use http::StatusCode;
//!
//! // String response
//! let response = "Hello, World!".into_response();
//!
//! // Status code with body
//! let response = (StatusCode::OK, "Success").into_response();
//!
//! // Empty response
//! let response = ().into_response();
//! ```

use std::{convert::Infallible, fmt::Display};

use bytes::Bytes;
use http::{HeaderName, HeaderValue, Response, StatusCode};
use http_body_util::Full;

use crate::body::TakoBody;

/// Trait for converting types into HTTP responses.
///
/// This trait provides a unified interface for converting various types into
/// `Response<TakoBody>` objects. It enables handlers to return different types
/// that are automatically converted to proper HTTP responses, making the API
/// more ergonomic and flexible.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use tako::body::TakoBody;
/// use http::Response;
///
/// // Custom implementation
/// struct JsonResponse {
///     data: String,
/// }
///
/// impl Responder for JsonResponse {
///     fn into_response(self) -> Response<TakoBody> {
///         let mut response = Response::new(TakoBody::from(self.data));
///         response.headers_mut().insert(
///             "content-type",
///             "application/json".parse().unwrap()
///         );
///         response
///     }
/// }
/// ```
pub trait Responder {
    /// Converts the implementing type into an HTTP response.
    fn into_response(self) -> Response<TakoBody>;
}

impl Responder for Response<TakoBody> {
    fn into_response(self) -> Response<TakoBody> {
        self
    }
}

impl Responder for &'static str {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from_static(
            self.as_bytes(),
        ))))
    }
}

impl Responder for String {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from(self))))
    }
}

impl Responder for () {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::empty())
    }
}

impl Responder for Infallible {
    fn into_response(self) -> Response<TakoBody> {
        match self {}
    }
}

pub struct StaticHeaders<const N: usize>(pub [(HeaderName, &'static str); N]);

impl<const N: usize> Responder for (StatusCode, StaticHeaders<N>) {
    fn into_response(self) -> Response<TakoBody> {
        let (status, StaticHeaders(headers)) = self;
        let mut res = Response::new(TakoBody::empty());
        *res.status_mut() = status;

        for (name, value) in headers {
            res.headers_mut()
                .append(name, HeaderValue::from_static(value));
        }
        res
    }
}

impl<R> Responder for (StatusCode, R)
where
    R: Display,
{
    fn into_response(self) -> Response<TakoBody> {
        let (status, body) = self;
        let mut res = Response::new(TakoBody::new(Full::from(Bytes::from(body.to_string()))));
        *res.status_mut() = status;
        res
    }
}

impl Responder for TakoBody {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(self)
    }
}

impl Responder for anyhow::Error {
    fn into_response(self) -> Response<TakoBody> {
        (StatusCode::BAD_REQUEST, self.to_string()).into_response()
    }
}

pub enum CompressionResponse<R>
where
    R: Responder,
{
    /// Plain, uncompressed response.
    Plain(R),
    /// Compressed or streaming response.
    Stream(R),
}

impl<R> Responder for CompressionResponse<R>
where
    R: Responder,
{
    fn into_response(self) -> Response<TakoBody> {
        match self {
            CompressionResponse::Plain(r) => r.into_response(),
            CompressionResponse::Stream(r) => r.into_response(),
        }
    }
}
