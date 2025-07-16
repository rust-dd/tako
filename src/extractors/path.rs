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

    /// Extracts the path from an HTTP request.
    ///
    /// Returns a wrapper around the request's URI path, providing access
    /// to the path string sent by the client.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, path::Path};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let Path(path) = Path::from_request(&mut req).await?;
    ///
    ///     // Route based on path
    ///     match path {
    ///         "/" => println!("Home page"),
    ///         "/about" => println!("About page"),
    ///         _ => println!("Path: {}", path),
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Path(req.uri().path())))
    }
}

impl<'a> FromRequestParts<'a> for Path<'a> {
    type Error = Infallible;

    /// Extracts the path from HTTP request parts.
    ///
    /// Returns a wrapper around the request parts' URI path, providing access
    /// to the path string sent by the client.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, path::Path};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let Path(path) = Path::from_request_parts(&mut parts).await?;
    ///
    ///     // Extract path segments for routing
    ///     let segments: Vec<&str> = path.split('/').skip(1).collect();
    ///
    ///     match segments.as_slice() {
    ///         ["api", "v1", "users"] => println!("Users API v1"),
    ///         ["api", "v1", "posts"] => println!("Posts API v1"),
    ///         _ => println!("Unmatched path: {}", path),
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Path(parts.uri.path())))
    }
}
