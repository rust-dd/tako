//! Header extraction from HTTP requests.
//!
//! This module provides the [`HeaderMap`](crate::extractors::header_map::HeaderMap) extractor for accessing HTTP headers from
//! incoming requests. It wraps a reference to the headers, allowing efficient access
//! to header values without copying the underlying data.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::header_map::HeaderMap;
//! use tako::types::Request;
//!
//! async fn handle_headers(HeaderMap(headers): HeaderMap<'_>) {
//!     // Check for specific headers
//!     if let Some(user_agent) = headers.get("user-agent") {
//!         println!("User-Agent: {:?}", user_agent);
//!     }
//!
//!     // Iterate over all headers
//!     for (name, value) in headers.iter() {
//!         println!("{}: {:?}", name, value);
//!     }
//! }
//! ```

use http::request::Parts;
use std::{convert::Infallible, future::ready};

use crate::{
  extractors::{FromRequest, FromRequestParts},
  types::Request,
};

/// Header map extractor that provides access to HTTP request headers.
///
/// This extractor wraps a reference to the headers of a request, providing
/// efficient access to header values without copying the underlying data.
/// It can be used to inspect, validate, or extract information from HTTP headers.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::header_map::HeaderMap;
/// use tako::types::Request;
///
/// async fn handler(HeaderMap(headers): HeaderMap<'_>) {
///     // Get authorization header
///     if let Some(auth) = headers.get("authorization") {
///         if let Ok(auth_str) = auth.to_str() {
///             println!("Authorization: {}", auth_str);
///         }
///     }
///
///     // Check content type
///     if let Some(content_type) = headers.get("content-type") {
///         println!("Content-Type: {:?}", content_type);
///     }
/// }
/// ```
#[doc(alias = "headers")]
pub struct HeaderMap<'a>(pub &'a http::HeaderMap);

impl<'a> FromRequest<'a> for HeaderMap<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(HeaderMap(req.headers())))
  }
}

impl<'a> FromRequestParts<'a> for HeaderMap<'a> {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(HeaderMap(&parts.headers)))
  }
}
