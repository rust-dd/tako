/// This module provides the `HeaderMap` extractor, which is used to extract the headers of a request.
use std::pin::Pin;

use anyhow::Result;
use http::request::Parts;

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

/// Implementation of the `FromRequest` trait for the `HeaderMap` extractor.
///
/// This allows the `HeaderMap` extractor to be used in request handlers to easily access
/// the headers of the request.
impl<'a> FromRequest<'a> for HeaderMap<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    /// Extracts the headers of the request.
    ///
    /// # Arguments
    ///
    /// * `req` - A mutable reference to the incoming request.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `Result` containing the `HeaderMap` extractor.
    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move { Ok(HeaderMap(req.headers())) })
    }
}

/// Implementation of the `FromRequestParts` trait for the `HeaderMap` extractor.
///
/// This allows the `HeaderMap` extractor to be used in request handlers to access
/// the headers from the `Parts` of a request.
impl<'a> FromRequestParts<'a> for HeaderMap<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    /// Extracts the headers from the `Parts` of a request.
    ///
    /// # Arguments
    ///
    /// * `parts` - A mutable reference to the `Parts` of the request.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `Result` containing the `HeaderMap` extractor.
    fn from_request_parts(parts: &'a mut Parts) -> Self::Fut {
        Box::pin(async move { Ok(HeaderMap(&parts.headers)) })
    }
}
