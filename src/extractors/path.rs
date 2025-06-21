/// This module provides the `Path` extractor, which is used to extract the path of a request.
use std::pin::Pin;

use anyhow::Result;
use http::request::Parts;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

/// The `Path` struct is an extractor that wraps a reference to the path of a request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::path::Path;
/// use tako::types::Request;
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let path = Path::from_request(&mut req).await?;
///     // Use the extracted path here
///     Ok(())
/// }
/// ```
pub struct Path<'a>(pub &'a str);

/// Implementation of the `FromRequest` trait for the `Path` extractor.
///
/// This allows the `Path` extractor to be used in request handlers to easily access
/// the path of the request.
impl<'a> FromRequest<'a> for Path<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(request: &'a mut Request) -> Self::Fut {
        Box::pin(async move { Ok(Path(request.uri().path())) })
    }
}

/// Implementation of the `FromRequestParts` trait for the `Path` extractor.
///
/// This allows the `Path` extractor to be used in request handlers to access
/// the path from the `Parts` of a request.
impl<'a> FromRequestParts<'a> for Path<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request_parts(parts: &'a mut Parts) -> Self::Fut {
        Box::pin(async move { Ok(Path(parts.uri.path())) })
    }
}
