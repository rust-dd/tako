/// This module provides the `Path` extractor, which is used to extract the path of a request.
use http::request::Parts;
use std::{convert::Infallible, future::ready};

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

impl<'a> FromRequest<'a> for Path<'a> {
    type Error = Infallible;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Path(req.uri().path())))
    }
}

impl<'a> FromRequestParts<'a> for Path<'a> {
    type Error = Infallible;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Path(parts.uri.path())))
    }
}
