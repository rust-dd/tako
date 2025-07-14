use hyper::body::Incoming;
/// This module provides the `Bytes` extractor, which is used to extract the body of a request as bytes.
use std::{convert::Infallible, future::ready};

use crate::{extractors::FromRequest, types::Request};

/// The `Bytes` struct is an extractor that wraps a reference to the incoming request body.
///
/// # Example
///
/// ```rust
/// use tako::extractors::bytes::Bytes;
/// use tako::types::Request;
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let bytes = Bytes::from_request(&mut req).await?;
///     // Use the extracted bytes here
///     Ok(())
/// }
/// ```
pub struct Bytes<'a>(pub &'a Incoming);

impl<'a> FromRequest<'a> for Bytes<'a> {
    type Error = Infallible;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Bytes(req.body())))
    }
}
