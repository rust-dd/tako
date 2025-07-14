/// This module provides traits and utilities for extracting data from HTTP requests.
///
/// It includes both synchronous and asynchronous mechanisms for extracting data from
/// various parts of an HTTP request, such as headers, query parameters, and request bodies.
use http::request::Parts;

/// Extractor for handling Accept-Language header in HTTP requests.
pub mod acc_lang;
/// Extractor for handling Basic authentication credentials in HTTP requests.
pub mod basic;
/// Extractor for handling Bearer tokens in HTTP requests.
pub mod bearer;
/// Extractor for handling raw byte data from HTTP request bodies.
pub mod bytes;
/// Extractor for handling cookie data from HTTP request headers.
pub mod cookie_jar;
/// Extractor for cookie key expansion and derivation.
pub mod cookie_key_expansion;
/// Extractor for handling private (encrypted) cookies.
pub mod cookie_private;
/// Extractor for handling signed cookies with HMAC verification.
pub mod cookie_signed;
/// Extractor for handling x-www-form-urlencoded data from HTTP request bodies.
pub mod form;
/// Extractor for working with HTTP headers as a map.
pub mod header_map;
/// Extractor for handling IP address data from HTTP request headers.
pub mod ipaddr;
/// Extractor for parsing JSON data from HTTP request bodies.
pub mod json;
/// Extractor for handling route or path parameters in HTTP requests.
pub mod params;
/// Extractor for working with the path component of HTTP requests.
pub mod path;
/// Extractor for parsing query parameters from HTTP requests.
pub mod query;

/// Extractor for handling multipart form data from HTTP request bodies.
#[cfg(feature = "multipart")]
pub mod multipart;

/// Extractor for parsing JSON data from HTTP request bodies accelerated by SIMD.
#[cfg(feature = "simd")]
pub mod simdjson;

/// The `FromRequest` trait provides a synchronous mechanism for extracting data from an HTTP request.
///
/// This trait is designed for types that need to synchronously extract and process data from the body,
/// headers, query parameters, or other components of an incoming HTTP request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
/// use anyhow::Result;
///
/// struct MyExtractor;
///
/// impl<'a> FromRequest<'a> for MyExtractor {
///     fn from_request(req: &'a Request) -> Result<Self> {
///         // Perform synchronous extraction
///         Ok(MyExtractor)
///     }
/// }
/// ```
pub trait FromRequest<'a>: Sized {
    /// Extractor-specific error which can be turned into an HTTP response.
    type Error: crate::responder::Responder;

    /// Perform the extraction. Synchronous extractors can return
    /// `core::future::ready(Ok(Self))`.
    fn from_request(
        req: &'a mut crate::types::Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}

/// The `FromRequestParts` trait provides a synchronous mechanism for extracting data from specific parts of an HTTP request.
///
/// This trait is designed for types that need to synchronously extract and process data from the headers,
/// URI, query parameters, or other metadata of an incoming HTTP request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::FromRequestParts;
/// use http::request::Parts;
/// use anyhow::Result;
///
/// struct MyPartsExtractor;
///
/// impl<'a> FromRequestParts<'a> for MyPartsExtractor {
///     fn from_request_parts(parts: &'a mut Parts) -> Result<Self> {
///         // Perform synchronous extraction from request parts
///         Ok(MyPartsExtractor)
///     }
/// }
/// ```
pub trait FromRequestParts<'a>: Sized {
    /// Extractor-specific error which can be turned into an HTTP response.
    type Error: crate::responder::Responder;

    /// Perform the extraction from request parts.
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a;
}
