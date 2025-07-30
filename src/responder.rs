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
use http_body_util::Full;
use hyper::{
    StatusCode,
    header::{HeaderName, HeaderValue},
};

use crate::{body::TakoBody, types::Response};

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
    fn into_response(self) -> Response;
}

impl Responder for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl Responder for TakoBody {
    fn into_response(self) -> Response {
        Response::new(self)
    }
}

impl Responder for &'static str {
    fn into_response(self) -> Response {
        Response::new(TakoBody::new(Full::from(Bytes::from_static(
            self.as_bytes(),
        ))))
    }
}

impl Responder for String {
    fn into_response(self) -> Response {
        Response::new(TakoBody::new(Full::from(Bytes::from(self))))
    }
}

impl Responder for () {
    fn into_response(self) -> Response {
        Response::new(TakoBody::empty())
    }
}

impl Responder for Infallible {
    fn into_response(self) -> Response {
        match self {}
    }
}

impl<R> Responder for (StatusCode, R)
where
    R: Display,
{
    fn into_response(self) -> Response {
        let (status, body) = self;
        let mut res = Response::new(TakoBody::new(Full::from(Bytes::from(body.to_string()))));
        *res.status_mut() = status;
        res
    }
}

pub struct StaticHeaders<const N: usize>(pub [(HeaderName, &'static str); N]);

impl<const N: usize> Responder for (StatusCode, StaticHeaders<N>) {
    fn into_response(self) -> Response {
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
