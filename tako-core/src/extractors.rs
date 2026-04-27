//! HTTP request data extraction utilities and traits.
//!
//! This module provides a comprehensive system for extracting data from HTTP requests in a
//! type-safe and ergonomic way. Extractors can parse various parts of requests including
//! headers, query parameters, JSON/form bodies, cookies, and path parameters. The module
//! defines two core traits: `FromRequest` for extractors that need access to the full
//! request (including body), and `FromRequestParts` for extractors that only need request
//! metadata like headers and URI.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::{FromRequest, FromRequestParts};
//! use tako::types::Request;
//! use http::request::Parts;
//! use anyhow::Result;
//!
//! // Simple header extractor
//! struct UserAgent(String);
//!
//! impl<'a> FromRequestParts<'a> for UserAgent {
//!     type Error = &'static str;
//!
//!     async fn from_request_parts(parts: &'a mut Parts) -> Result<Self, Self::Error> {
//!         let user_agent = parts.headers
//!             .get("user-agent")
//!             .and_then(|v| v.to_str().ok())
//!             .unwrap_or("unknown");
//!         Ok(UserAgent(user_agent.to_string()))
//!     }
//! }
//! ```

use http::request::Parts;

/// Checks if the Content-Type header indicates JSON content.
#[doc(hidden)]
pub fn is_json_content_type(headers: &http::HeaderMap) -> bool {
  headers
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|ct| ct.parse::<mime_guess::Mime>().ok())
    .map(|mime| {
      mime.type_() == "application"
        && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
    })
    .unwrap_or(false)
}

/// JSON request body parsing and deserialization.
pub mod json;

/// Path parameter extraction from dynamic route segments.
pub mod params;

/// Range header parsing for partial content requests.
pub mod range;

/// Compile-time-shaped path parameter extractor (paired with `#[tako::route]`).
pub mod typed_params;

/// Trait for extracting data from complete HTTP requests.
///
/// `FromRequest` enables types to extract and parse data from HTTP requests, including
/// access to the request body. This trait is designed for extractors that need to consume
/// or parse the request body, such as JSON deserializers, form parsers, or raw byte
/// extractors. The extraction is asynchronous to support streaming body processing.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct CreateUser {
///     name: String,
///     email: String,
/// }
///
/// // Custom JSON extractor implementation
/// impl<'a> FromRequest<'a> for CreateUser {
///     type Error = &'static str;
///
///     async fn from_request(req: &'a mut Request) -> Result<Self, Self::Error> {
///         // In a real implementation, this would parse JSON from the request body
///         Ok(CreateUser {
///             name: "John Doe".to_string(),
///             email: "john@example.com".to_string(),
///         })
///     }
/// }
/// ```
pub trait FromRequest<'a>: Sized {
  /// Error type returned when extraction fails.
  type Error: crate::responder::Responder;

  /// Extracts the type from the HTTP request.
  fn from_request(
    req: &'a mut crate::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}

/// Trait for extracting data from HTTP request parts (metadata only).
///
/// `FromRequestParts` enables types to extract data from request metadata such as
/// headers, URI, method, and extensions, without needing access to the request body.
/// This is more efficient for extractors that only need metadata and allows multiple
/// extractors to be used on the same request since the body is not consumed.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::FromRequestParts;
/// use http::request::Parts;
/// use http::Method;
///
/// struct RequestMethod(Method);
///
/// impl<'a> FromRequestParts<'a> for RequestMethod {
///     type Error = &'static str;
///
///     async fn from_request_parts(parts: &'a mut Parts) -> Result<Self, Self::Error> {
///         Ok(RequestMethod(parts.method.clone()))
///     }
/// }
///
/// // Usage in a handler
/// async fn handler(method: RequestMethod) -> String {
///     format!("Request method: {}", method.0)
/// }
/// ```
pub trait FromRequestParts<'a>: Sized {
  /// Error type returned when extraction fails.
  type Error: crate::responder::Responder;

  /// Extracts the type from the HTTP request parts.
  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}

// Built-in extractor for borrowing the request itself in handlers: `&mut Request`.
// This enables signatures like `async fn handler(req: &mut Request, Path(..), ...)`.
impl<'a> FromRequest<'a> for &'a mut crate::types::Request {
  type Error = core::convert::Infallible;

  fn from_request(
    req: &'a mut crate::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(req))
  }
}
