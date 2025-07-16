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

/// Accept-Language header parsing and locale extraction.
pub mod acc_lang;

/// Basic HTTP authentication credential extraction.
pub mod basic;

/// Bearer token authentication extraction from Authorization header.
pub mod bearer;

/// Raw byte data extraction from request bodies.
pub mod bytes;

/// Cookie parsing and management utilities.
pub mod cookie_jar;

/// Cookie key derivation and expansion for encryption/signing.
pub mod cookie_key_expansion;

/// Private (encrypted) cookie handling with automatic decryption.
pub mod cookie_private;

/// Signed cookie handling with HMAC verification.
pub mod cookie_signed;

/// Form data (application/x-www-form-urlencoded) parsing.
pub mod form;

/// HTTP header map extraction and manipulation.
pub mod header_map;

/// IP address extraction from request headers and connection info.
pub mod ipaddr;

/// JSON request body parsing and deserialization.
pub mod json;

/// Path parameter extraction from dynamic route segments.
pub mod params;

/// URL path component extraction and manipulation.
pub mod path;

/// Query parameter parsing from URL query strings.
pub mod query;

/// Multipart form data parsing for file uploads and complex forms.
#[cfg(feature = "multipart")]
pub mod multipart;

/// High-performance JSON parsing using SIMD acceleration.
#[cfg(feature = "simd")]
pub mod simdjson;

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
    ///
    /// The error type must implement `Responder` so it can be converted into an
    /// HTTP response when extraction fails, allowing for custom error handling
    /// and user-friendly error messages.
    type Error: crate::responder::Responder;

    /// Extracts the type from the HTTP request.
    ///
    /// This method performs the actual extraction logic, which may involve parsing
    /// the request body, validating headers, or other request processing. The method
    /// is asynchronous to support streaming body parsing and other async operations.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    /// use tako::body::TakoBody;
    ///
    /// struct TextBody(String);
    ///
    /// impl<'a> FromRequest<'a> for TextBody {
    ///     type Error = &'static str;
    ///
    ///     async fn from_request(req: &'a mut Request) -> Result<Self, Self::Error> {
    ///         // In a real implementation, this would read the request body
    ///         Ok(TextBody("Hello, World!".to_string()))
    ///     }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut request = Request::builder().body(TakoBody::empty())?;
    /// let text_body = TextBody::from_request(&mut request).await.unwrap();
    /// assert_eq!(text_body.0, "Hello, World!");
    /// # Ok(())
    /// # }
    /// ```
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
    ///
    /// The error type must implement `Responder` so it can be converted into an
    /// HTTP response when extraction fails, enabling custom error responses and
    /// proper error handling in the application.
    type Error: crate::responder::Responder;

    /// Extracts the type from the HTTP request parts.
    ///
    /// This method performs extraction using only the request metadata, making it
    /// suitable for extractors that work with headers, query parameters, path
    /// parameters, or other non-body data. Multiple `FromRequestParts` extractors
    /// can be used in the same handler since they don't consume the request body.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::FromRequestParts;
    /// use http::request::Parts;
    /// use http::{Method, Uri};
    ///
    /// struct RequestInfo {
    ///     method: Method,
    ///     path: String,
    /// }
    ///
    /// impl<'a> FromRequestParts<'a> for RequestInfo {
    ///     type Error = &'static str;
    ///
    ///     async fn from_request_parts(parts: &'a mut Parts) -> Result<Self, Self::Error> {
    ///         Ok(RequestInfo {
    ///             method: parts.method.clone(),
    ///             path: parts.uri.path().to_string(),
    ///         })
    ///     }
    /// }
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let mut parts = Parts {
    ///     method: Method::GET,
    ///     uri: "/users/123".parse()?,
    ///     version: http::Version::HTTP_11,
    ///     headers: Default::default(),
    ///     extensions: Default::default(),
    /// };
    ///
    /// let info = RequestInfo::from_request_parts(&mut parts).await.unwrap();
    /// assert_eq!(info.method, Method::GET);
    /// assert_eq!(info.path, "/users/123");
    /// # Ok(())
    /// # }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}
