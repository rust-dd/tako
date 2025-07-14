/// This module provides the `HeaderMap` extractor, which is used to extract the headers of a request.
use http::request::Parts;
use std::{convert::Infallible, future::ready};

use crate::{
    extractors::{FromRequest, FromRequestParts},
    types::Request,
};

/// The `HeaderMap` struct is an extractor that wraps a reference to the headers of a request.
///
/// # Example
///
/// ```rust
/// use tako::extractors::header_map::HeaderMap;
/// use tako::types::Request;
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let headers = HeaderMap::from_request(&mut req).await?;
///     // Use the extracted headers here
///     Ok(())
/// }
/// ```
pub struct HeaderMap<'a>(pub &'a hyper::HeaderMap);

impl<'a> FromRequest<'a> for HeaderMap<'a> {
    type Error = Infallible;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(HeaderMap(req.headers())))
    }
}

impl<'a> FromRequestParts<'a> for HeaderMap<'a> {
    type Error = Infallible;

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(HeaderMap(&parts.headers)))
    }
}
