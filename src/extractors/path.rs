//! Path extraction from HTTP requests.
//!
//! This module provides the [`Path`] extractor for accessing the URI path from
//! incoming HTTP requests. It wraps a reference to the path string, allowing
//! efficient access to the request path without copying the underlying data.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::path::Path;
//! use tako::types::Request;
//!
//! async fn handle_path(Path(path): Path<'_>) {
//!     println!("Request path: {}", path);
//!
//!     // Check specific path patterns
//!     if path.starts_with("/api/") {
//!         println!("API endpoint");
//!     }
//!
//!     // Extract path segments
//!     let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
//!     println!("Path segments: {:?}", segments);
//! }
//! ```

use http::request::Parts;
use std::{convert::Infallible, future::ready};

use crate::{
  extractors::{FromRequest, FromRequestParts},
  types::Request,
};

/// Path extractor that provides access to the HTTP request path.
///
/// This extractor wraps a reference to the URI path of a request, providing
/// efficient access to the path string without copying the underlying data.
/// It can be used to inspect, validate, or extract information from the request path.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::path::Path;
/// use tako::types::Request;
///
/// async fn handler(Path(path): Path<'_>) {
///     match path {
///         "/health" => println!("Health check endpoint"),
///         "/api/users" => println!("Users API endpoint"),
///         _ if path.starts_with("/static/") => println!("Static file request"),
///         _ => println!("Other path: {}", path),
///     }
/// }
/// ```
pub struct Path<'a>(pub &'a str);

impl<'a> FromRequest<'a> for Path<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(Path(req.uri().path())))
  }
}

impl<'a> FromRequestParts<'a> for Path<'a> {
  type Error = Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Ok(Path(parts.uri.path())))
  }
}
