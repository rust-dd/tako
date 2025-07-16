//! Header extraction from HTTP requests.
//!
//! This module provides the [`HeaderMap`] extractor for accessing HTTP headers from
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
pub struct HeaderMap<'a>(pub &'a hyper::HeaderMap);

impl<'a> FromRequest<'a> for HeaderMap<'a> {
    type Error = Infallible;

    /// Extracts headers from an HTTP request.
    ///
    /// Returns a wrapper around the request's header map, providing access
    /// to all HTTP headers sent by the client.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, header_map::HeaderMap};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let HeaderMap(headers) = HeaderMap::from_request(&mut req).await?;
    ///
    ///     // Access specific headers
    ///     if let Some(host) = headers.get("host") {
    ///         println!("Host: {:?}", host);
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(HeaderMap(req.headers())))
    }
}

impl<'a> FromRequestParts<'a> for HeaderMap<'a> {
    type Error = Infallible;

    /// Extracts headers from HTTP request parts.
    ///
    /// Returns a wrapper around the request parts' header map, providing access
    /// to all HTTP headers sent by the client.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, header_map::HeaderMap};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let HeaderMap(headers) = HeaderMap::from_request_parts(&mut parts).await?;
    ///
    ///     // Check for custom headers
    ///     if let Some(api_key) = headers.get("x-api-key") {
    ///         println!("API Key present");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(HeaderMap(&parts.headers)))
    }
}
