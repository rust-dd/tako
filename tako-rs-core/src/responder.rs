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
    new_response(self)
  }
}

impl Responder for &'static str {
  fn into_response(self) -> Response {
    content_response(
      TakoBody::from(Bytes::from_static(self.as_bytes())),
      "text/plain; charset=utf-8",
    )
  }
}

impl Responder for String {
  fn into_response(self) -> Response {
    content_response(TakoBody::from(self), "text/plain; charset=utf-8")
  }
}

impl Responder for () {
  fn into_response(self) -> Response {
    new_response(TakoBody::empty())
  }
}

impl Responder for Infallible {
  fn into_response(self) -> Response {
    match self {}
  }
}

impl<R: Responder> Responder for (StatusCode, R) {
  fn into_response(self) -> Response {
    let mut response = self.1.into_response();
    *response.status_mut() = self.0;
    response
  }
}

impl Responder for Bytes {
  fn into_response(self) -> Response {
    content_response(TakoBody::from(self), "application/octet-stream")
  }
}

impl Responder for Vec<u8> {
  fn into_response(self) -> Response {
    Bytes::from(self).into_response()
  }
}

impl Responder for Cow<'static, str> {
  fn into_response(self) -> Response {
    match self {
      Cow::Borrowed(s) => s.into_response(),
      Cow::Owned(s) => s.into_response(),
    }
  }
}

impl Responder for serde_json::Value {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self) {
      Ok(buf) => {
        let mut res = new_response(TakoBody::full(Full::from(Bytes::from(buf))));
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

impl Responder for HeaderMap {
  fn into_response(self) -> Response {
    let mut res = Response::new(TakoBody::empty());
    *res.headers_mut() = self;
    res
  }
}

impl Responder for StatusCode {
  fn into_response(self) -> Response {
    let mut res = new_response(TakoBody::empty());
    *res.status_mut() = self;
    res
  }
}

pub struct StaticHeaders<const N: usize>(pub [(HeaderName, &'static str); N]);

impl<const N: usize> Responder for StaticHeaders<N> {
  fn into_response(self) -> Response {
    let mut response = ().into_response();
    for (name, value) in self.0 {
      response
        .headers_mut()
        .append(name, HeaderValue::from_static(value));
    }
    response
  }
}

impl Responder for anyhow::Error {
  fn into_response(self) -> Response {
    tracing::error!(error = ?self, "request handler failed");
    (StatusCode::INTERNAL_SERVER_ERROR, "Internal Server Error").into_response()
  }
}

/// Converts either branch through its own [`Responder`] implementation.
impl<T: Responder, E: Responder> Responder for Result<T, E> {
  fn into_response(self) -> Response {
    match self {
      Ok(value) => value.into_response(),
      Err(error) => error.into_response(),
    }
  }
}

/// Compatibility marker; all [`Responder`] types can be returned as errors.
#[deprecated(note = "Result errors only need to implement Responder")]
pub trait ResponderError: Responder {}

/// Builds a response on a header map recycled from an earlier request.
pub(crate) fn new_response(body: TakoBody) -> Response {
  let mut response = Response::new(body);
  *response.headers_mut() = crate::recycle::take_headers();
  response
}

fn content_response(body: TakoBody, content_type: &'static str) -> Response {
  let mut response = new_response(body);
  response.headers_mut().insert(
    http::header::CONTENT_TYPE,
    HeaderValue::from_static(content_type),
  );
  response
}
