use anyhow::Result;
use http::request::Parts;

use crate::types::Request;

pub mod bytes;
pub mod header_map;
pub mod json;
pub mod params;
pub mod path;
pub mod query;

/// The `FromRequest` trait defines an interface for extracting data from an HTTP request.
///
/// This trait is implemented by types that need to extract and process data from the body,
/// headers, or other parts of an incoming HTTP request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
/// use anyhow::Result;
/// use std::pin::Pin;
///
/// struct MyExtractor;
///
/// impl<'a> FromRequest<'a> for MyExtractor {
///     type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;
///
///     fn from_request(req: &'a mut Request) -> Self::Fut {
///         Box::pin(async move {
///             // Extract data from the request
///             Ok(MyExtractor)
///         })
///     }
/// }
/// ```
pub trait FromRequest<'a>: Sized {
    type Fut: Future<Output = Result<Self>> + Send + 'a;

    fn from_request(req: &'a mut Request) -> Self::Fut;
}

/// The `FromRequestParts` trait defines an interface for extracting data from the parts of an HTTP request.
///
/// This trait is implemented by types that need to extract and process data from the headers,
/// URI, or other metadata of an incoming HTTP request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::FromRequestParts;
/// use http::request::Parts;
/// use anyhow::Result;
/// use std::pin::Pin;
///
/// struct MyPartsExtractor;
///
/// impl<'a> FromRequestParts<'a> for MyPartsExtractor {
///     type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;
///
///     fn from_request_parts(parts: &'a mut Parts) -> Self::Fut {
///         Box::pin(async move {
///             // Extract data from the request parts
///             Ok(MyPartsExtractor)
///         })
///     }
/// }
/// ```
pub trait FromRequestParts<'a>: Sized {
    type Fut: Future<Output = Result<Self>> + Send + 'a;

    fn from_request_parts(parts: &'a mut Parts) -> Self::Fut;
}
