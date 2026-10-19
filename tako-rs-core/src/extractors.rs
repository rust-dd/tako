//! [`FromRequest`] and [`FromRequestParts`], plus the extractors whose types the router uses.
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

/// Whether `Content-Type` is `application/json` or an `application/*+json` type.
#[doc(hidden)]
pub fn is_json_content_type(headers: &http::HeaderMap) -> bool {
  headers
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .and_then(|ct| ct.parse::<mime_guess::Mime>().ok())
    .is_some_and(|mime| {
      mime.type_() == "application"
        && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
    })
}

pub mod body;
pub mod json;
pub mod params;
pub mod range;
pub mod typed_params;

/// Request extensions the router provides; a route without middleware attaches only those
/// its handler's extractors list in `ENTRIES`, the full pipeline all of them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Entries(u8);

impl Entries {
  pub const NONE: Self = Self(0);
  /// [`ConnInfo`](crate::conn_info::ConnInfo) and the peer `SocketAddr`.
  pub const CONN: Self = Self(1);
  /// [`MatchedPath`](crate::router_state::MatchedPath).
  pub const MATCHED_PATH: Self = Self(1 << 1);
  /// The matched path parameters.
  pub const PARAMS: Self = Self(1 << 2);
  /// The router or route state that `State<T>` reads.
  pub const STATE: Self = Self(1 << 3);
  /// The body limit that body extractors enforce.
  pub const BODY_LIMIT: Self = Self(1 << 4);
  /// The route's SIMD JSON mode.
  pub const SIMD_JSON: Self = Self(1 << 5);
  /// Every entry; the default for extractors that list none.
  pub const ALL: Self = Self(0b11_1111);

  /// The entries in either set.
  #[must_use]
  pub const fn union(self, other: Self) -> Self {
    Self(self.0 | other.0)
  }

  /// Whether every entry in `other` is also in `self`.
  #[must_use]
  pub const fn contains(self, other: Self) -> bool {
    self.0 & other.0 == other.0
  }
}

/// Extracts a value from the whole request, body included; a handler's last argument is one.
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
  /// Sent as the response, instead of calling the handler, when extraction fails.
  type Error: crate::responder::Responder;

  /// Entries this extractor reads; the default, [`Entries::ALL`], is always correct, while
  /// an entry left out is missing on routes without middleware.
  const ENTRIES: Entries = Entries::ALL;

  fn from_request(
    req: &'a mut crate::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}

/// Extracts a value from the request head; every handler argument but the last is one.
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
  /// Sent as the response, instead of calling the handler, when extraction fails.
  type Error: crate::responder::Responder;

  /// Entries this extractor reads; the default, [`Entries::ALL`], is always correct, while
  /// an entry left out is missing on routes without middleware.
  const ENTRIES: Entries = Entries::ALL;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}
