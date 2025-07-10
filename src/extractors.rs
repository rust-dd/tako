/// This module provides traits and utilities for extracting data from HTTP requests.
///
/// It includes both synchronous and asynchronous mechanisms for extracting data from
/// various parts of an HTTP request, such as headers, query parameters, and request bodies.
use anyhow::Result;
use http::request::Parts;

use crate::types::Request;

/// Extractor for handling Basic authentication credentials in HTTP requests.
pub mod basic;
/// Extractor for handling Bearer tokens in HTTP requests.
pub mod bearer;
/// Extractor for handling raw byte data from HTTP request bodies.
pub mod bytes;
/// Extractor for handling x-www-form-urlencoded data from HTTP request bodies.
pub mod form;
/// Extractor for working with HTTP headers as a map.
pub mod header_map;
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
    fn from_request(req: &'a Request) -> Result<Self>;
}

pub trait FromRequestMut<'a>: Sized {
    fn from_request(req: &'a mut Request) -> Result<Self>;
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
    fn from_request_parts(parts: &'a mut Parts) -> Result<Self>;
}

/// The `AsyncFromRequest` trait provides an asynchronous mechanism for extracting data from an HTTP request.
///
/// This trait is designed for types that need to perform asynchronous operations while extracting
/// and processing data from the body, headers, query parameters, or other components of an incoming HTTP request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::AsyncFromRequest;
/// use tako::types::Request;
/// use anyhow::Result;
/// use std::future::Future;
///
/// struct MyAsyncExtractor;
///
/// impl<'a> AsyncFromRequest<'a> for MyAsyncExtractor {
///     fn from_request(req: &'a Request) -> impl Future<Output = Result<Self>> {
///         async move {
///             // Perform asynchronous extraction
///             Ok(MyAsyncExtractor)
///         }
///     }
/// }
/// ```
pub trait AsyncFromRequest<'a>: Sized {
    fn from_request(req: &'a Request) -> impl Future<Output = Result<Self>>;
}

/// The `AsyncFromRequestMut` trait provides an asynchronous mechanism for extracting data from a mutable HTTP request.
///
/// This trait is designed for types that need to perform asynchronous operations while extracting
/// and processing data from the body, headers, query parameters, or other components of an incoming mutable HTTP request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::AsyncFromRequestMut;
/// use tako::types::Request;
/// use anyhow::Result;
/// use std::future::Future;
///
/// struct MyAsyncMutExtractor;
///
/// impl<'a> AsyncFromRequestMut<'a> for MyAsyncMutExtractor {
///     fn from_request(req: &'a mut Request) -> impl Future<Output = Result<Self>> {
///         async move {
///             // Perform asynchronous extraction
///             Ok(MyAsyncMutExtractor)
///         }
///     }
/// }
/// ```
pub trait AsyncFromRequestMut<'a>: Sized {
    fn from_request(req: &'a mut Request) -> impl Future<Output = Result<Self>>;
}
