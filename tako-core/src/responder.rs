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

use std::borrow::Cow;
use std::convert::Infallible;

use bytes::Bytes;
use http::HeaderMap;
use http::StatusCode;
use http::header::HeaderName;
use http::header::HeaderValue;
use http_body_util::Full;

use crate::body::TakoBody;
use crate::types::Response;

/// A default 404 Not Found response.
///
/// Useful as a simple fallback:
/// `router.fallback(|_| async { NOT_FOUND });`
pub const NOT_FOUND: (StatusCode, &str) = (StatusCode::NOT_FOUND, "Not Found");

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
#[doc(alias = "response")]
pub trait Responder {
  /// Converts the implementing type into an HTTP response.
  fn into_response(self) -> Response;
}

/// Alias for [`Responder`] matching the axum-style naming.
///
/// Both names refer to the same trait; pick whichever reads better in context.
/// Existing code using `Responder` continues to compile unchanged.
pub use Responder as IntoResponse;

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
    Response::new(TakoBody::full(Full::from(Bytes::from_static(
      self.as_bytes(),
    ))))
  }
}

impl Responder for String {
  fn into_response(self) -> Response {
    Response::new(TakoBody::full(Full::from(Bytes::from(self))))
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

impl Responder for (StatusCode, &'static str) {
  fn into_response(self) -> Response {
    let (status, body) = self;
    let mut res = Response::new(TakoBody::full(Full::from(Bytes::from_static(
      body.as_bytes(),
    ))));
    *res.status_mut() = status;
    res
  }
}

impl Responder for (StatusCode, String) {
  fn into_response(self) -> Response {
    let (status, body) = self;
    let mut res = Response::new(TakoBody::full(Full::from(Bytes::from(body))));
    *res.status_mut() = status;
    res
  }
}

impl Responder for (StatusCode, Vec<u8>) {
  fn into_response(self) -> Response {
    let (status, body) = self;
    let mut res = Response::new(TakoBody::full(Full::from(Bytes::from(body))));
    *res.status_mut() = status;
    res
  }
}

impl Responder for Bytes {
  fn into_response(self) -> Response {
    Response::new(TakoBody::full(Full::from(self)))
  }
}

impl Responder for Vec<u8> {
  fn into_response(self) -> Response {
    Response::new(TakoBody::full(Full::from(Bytes::from(self))))
  }
}

impl Responder for Cow<'static, str> {
  fn into_response(self) -> Response {
    match self {
      Cow::Borrowed(s) => Response::new(TakoBody::full(Full::from(Bytes::from_static(s.as_bytes())))),
      Cow::Owned(s) => Response::new(TakoBody::full(Full::from(Bytes::from(s)))),
    }
  }
}

impl Responder for serde_json::Value {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::full(Full::from(Bytes::from(buf))));
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => {
        let mut res = Response::new(TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}

impl Responder for (StatusCode, HeaderMap, TakoBody) {
  fn into_response(self) -> Response {
    let (status, headers, body) = self;
    let mut res = Response::new(body);
    *res.status_mut() = status;
    *res.headers_mut() = headers;
    res
  }
}

impl Responder for (StatusCode, HeaderMap) {
  fn into_response(self) -> Response {
    let (status, headers) = self;
    let mut res = Response::new(TakoBody::empty());
    *res.status_mut() = status;
    *res.headers_mut() = headers;
    res
  }
}

impl Responder for HeaderMap {
  fn into_response(self) -> Response {
    let mut res = Response::new(TakoBody::empty());
    *res.headers_mut() = self;
    res
  }
}

impl Responder for StatusCode {
  fn into_response(self) -> Response {
    let mut res = Response::new(TakoBody::empty());
    *res.status_mut() = self;
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
      res
        .headers_mut()
        .append(name, HeaderValue::from_static(value));
    }
    res
  }
}

impl<T> Responder for anyhow::Result<T>
where
  T: Responder,
{
  fn into_response(self) -> Response {
    match self {
      Ok(ok) => ok.into_response(),
      Err(err) => {
        let mut res = Response::new(TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}

/// Native `Result<R, E>` support for handler returns where both arms implement
/// [`Responder`]. The `Ok` value renders normally; the `Err` value is rendered
/// via its own [`Responder`] impl so error types stay typed instead of being
/// forced through a single panic-or-string path.
impl<T, E> Responder for Result<T, E>
where
  T: Responder,
  E: ResponderError,
{
  fn into_response(self) -> Response {
    match self {
      Ok(ok) => ok.into_response(),
      Err(err) => err.into_response(),
    }
  }
}

/// Marker trait that opts a type into being used as the `Err` arm of a
/// handler-returned `Result<_, E>`.
///
/// Implement [`Responder`] on your error type, then add `impl ResponderError for MyErr {}`
/// to make it usable as `Result<_, MyErr>`. The marker prevents the blanket
/// `Result<_, E>` impl from colliding with [`Responder for anyhow::Result<T>`]
/// — `anyhow::Error` does not implement `ResponderError`, so the dedicated
/// `anyhow::Result<T>` impl above keeps applying to `anyhow`-flavoured handlers.
pub trait ResponderError: Responder {}
