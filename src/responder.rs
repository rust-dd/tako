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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::responder::Responder;
    ///
    /// let response = "Hello, World!".into_response();
    /// assert_eq!(response.status(), 200);
    /// ```
    fn into_response(self) -> Response<TakoBody>;
}

/// Converts an existing `Response<TakoBody>` into itself.
///
/// This implementation allows `Response<TakoBody>` to be used directly as a
/// responder without any conversion, providing a pass-through for pre-built responses.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use tako::body::TakoBody;
/// use http::Response;
///
/// let response = Response::new(TakoBody::empty());
/// let converted = response.into_response();
/// ```
impl Responder for Response<TakoBody> {
    fn into_response(self) -> Response<TakoBody> {
        self
    }
}

/// Converts a static string slice into a plain text HTTP response.
///
/// Creates an HTTP response with the string as the body content. The response
/// uses the default status code (200 OK) and no special content-type headers.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
///
/// let response = "Hello, World!".into_response();
/// assert_eq!(response.status(), 200);
/// ```
impl Responder for &'static str {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from_static(
            self.as_bytes(),
        ))))
    }
}

/// Converts a `String` into a plain text HTTP response.
///
/// Creates an HTTP response with the string content as the body. The string
/// is consumed and converted into bytes for the response body.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
///
/// let message = String::from("Dynamic content");
/// let response = message.into_response();
/// assert_eq!(response.status(), 200);
/// ```
impl Responder for String {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::new(Full::from(Bytes::from(self))))
    }
}

/// Converts the unit type into an empty HTTP response.
///
/// Creates an HTTP response with no body content, useful for endpoints that
/// only need to indicate success without returning data.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
///
/// let response = ().into_response();
/// assert_eq!(response.status(), 200);
/// ```
impl Responder for () {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(TakoBody::empty())
    }
}

/// Converts `Infallible` into an HTTP response.
///
/// Since `Infallible` can never be instantiated, this implementation is
/// unreachable but required for type system completeness.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use std::convert::Infallible;
///
/// // This would never actually execute since Infallible cannot be created
/// fn unreachable_example(never: Infallible) {
///     let _response = never.into_response();
/// }
/// ```
impl Responder for Infallible {
    fn into_response(self) -> Response<TakoBody> {
        match self {}
    }
}

/// Wrapper for static header name-value pairs.
///
/// This struct holds an array of header name-value pairs where the values
/// are static string slices, allowing for efficient header construction
/// without heap allocation.
///
/// # Examples
///
/// ```rust
/// use tako::responder::{Responder, StaticHeaders};
/// use http::{HeaderName, StatusCode};
///
/// let headers = StaticHeaders([
///     (HeaderName::from_static("content-type"), "application/json"),
///     (HeaderName::from_static("cache-control"), "no-cache"),
/// ]);
/// let response = (StatusCode::OK, headers).into_response();
/// ```
pub struct StaticHeaders<const N: usize>(pub [(HeaderName, &'static str); N]);

/// Converts a status code and static headers into an HTTP response.
///
/// Creates an empty response with the specified status code and headers.
/// The headers are added to the response using static string values for
/// efficient memory usage.
///
/// # Examples
///
/// ```rust
/// use tako::responder::{Responder, StaticHeaders};
/// use http::{HeaderName, StatusCode};
///
/// let headers = StaticHeaders([
///     (HeaderName::from_static("content-type"), "application/json"),
/// ]);
/// let response = (StatusCode::CREATED, headers).into_response();
/// assert_eq!(response.status(), StatusCode::CREATED);
/// ```
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

/// Converts a status code and displayable body into an HTTP response.
///
/// Creates an HTTP response with the specified status code and the string
/// representation of the body content. The body is converted using the
/// `Display` trait implementation.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use http::StatusCode;
///
/// let response = (StatusCode::NOT_FOUND, "Resource not found").into_response();
/// assert_eq!(response.status(), StatusCode::NOT_FOUND);
///
/// let response = (StatusCode::OK, 42).into_response();
/// assert_eq!(response.status(), StatusCode::OK);
/// ```
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

/// Converts a `TakoBody` directly into an HTTP response.
///
/// Creates an HTTP response using the provided body content directly,
/// with default status code and headers.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use tako::body::TakoBody;
///
/// let body = TakoBody::empty();
/// let response = body.into_response();
/// assert_eq!(response.status(), 200);
/// ```
impl Responder for TakoBody {
    fn into_response(self) -> Response<TakoBody> {
        Response::new(self)
    }
}

/// Converts an `anyhow::Error` into a bad request HTTP response.
///
/// Creates an HTTP response with a 400 Bad Request status and the error
/// message as the response body, useful for error handling in handlers.
///
/// # Examples
///
/// ```rust
/// use tako::responder::Responder;
/// use anyhow::anyhow;
/// use http::StatusCode;
///
/// let error = anyhow!("Something went wrong");
/// let response = error.into_response();
/// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
/// ```
impl Responder for anyhow::Error {
    fn into_response(self) -> Response<TakoBody> {
        (StatusCode::BAD_REQUEST, self.to_string()).into_response()
    }
}

/// Enum for handling compressed and uncompressed response variants.
///
/// This enum allows handlers to return either plain responses or compressed
/// responses, providing flexibility in response handling based on client
/// capabilities or content characteristics.
///
/// # Examples
///
/// ```rust
/// use tako::responder::{Responder, CompressionResponse};
///
/// let plain_response = CompressionResponse::Plain("Hello");
/// let compressed_response = CompressionResponse::Stream("Large content");
///
/// let response1 = plain_response.into_response();
/// let response2 = compressed_response.into_response();
/// ```
pub enum CompressionResponse<R>
where
    R: Responder,
{
    /// Plain, uncompressed response.
    Plain(R),
    /// Compressed or streaming response.
    Stream(R),
}

/// Converts a `CompressionResponse` into an HTTP response.
///
/// This implementation handles both plain and streaming response variants
/// by delegating to the underlying responder's `into_response` method.
///
/// # Examples
///
/// ```rust
/// use tako::responder::{Responder, CompressionResponse};
///
/// let response = CompressionResponse::Plain("Hello, World!");
/// let http_response = response.into_response();
/// assert_eq!(http_response.status(), 200);
///
/// let response = CompressionResponse::Stream("Streaming content");
/// let http_response = response.into_response();
/// assert_eq!(http_response.status(), 200);
/// ```
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
