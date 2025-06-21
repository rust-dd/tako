/// This module provides the `Bytes` extractor, which is used to extract the body of a request as bytes.
use std::pin::Pin;

use anyhow::Result;
use hyper::body::Incoming;

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

/// Implementation of the `FromRequest` trait for the `Bytes` extractor.
///
/// This allows the `Bytes` extractor to be used in request handlers to easily access
/// the body of the request as bytes.
impl<'a> FromRequest<'a> for Bytes<'a> {
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    /// Extracts the body of the request as bytes.
    ///
    /// # Arguments
    ///
    /// * `req` - A mutable reference to the incoming request.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `Result` containing the `Bytes` extractor.
    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move { Ok(Bytes(req.body())) })
    }
}
