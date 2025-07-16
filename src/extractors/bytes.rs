//! Raw request body access for HTTP requests.
//!
//! This module provides the [`Bytes`] extractor for accessing the raw HTTP request body
//! as a `hyper::body::Incoming` stream. This is useful when you need low-level access
//! to the request body stream for custom processing, streaming, or when working directly
//! with hyper's body types.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::bytes::Bytes;
//! use tako::types::Request;
//! use http_body_util::BodyExt;
//!
//! async fn handle_raw_body(Bytes(body): Bytes<'_>) {
//!     // Access the raw hyper body stream
//!     println!("Got access to raw body stream");
//!
//!     // You can use hyper's body utilities to read the body
//!     // let full_body = body.collect().await.unwrap();
//!     // let bytes = full_body.to_bytes();
//! }
//! ```

use hyper::body::Incoming;
use std::{convert::Infallible, future::ready};

use crate::{extractors::FromRequest, types::Request};

/// Raw request body extractor that provides access to the underlying body stream.
///
/// This extractor wraps a reference to the raw `hyper::body::Incoming` stream,
/// allowing direct access to the request body without any processing or buffering.
/// It's useful for scenarios where you need:
///
/// - Streaming request body processing
/// - Custom body parsing logic
/// - Direct integration with hyper's body APIs
/// - Memory-efficient handling of large request bodies
///
/// # Examples
///
/// ```rust
/// use tako::extractors::bytes::Bytes;
/// use tako::types::Request;
/// use http_body_util::BodyExt;
///
/// async fn stream_handler(Bytes(body): Bytes<'_>) {
///     // Process body as a stream
///     println!("Processing request body stream");
///
///     // Example: collect entire body into bytes
///     // let collected = body.collect().await.unwrap();
///     // let bytes = collected.to_bytes();
///     // println!("Body size: {} bytes", bytes.len());
/// }
///
/// async fn chunked_handler(Bytes(mut body): Bytes<'_>) {
///     // Process body in chunks
///     // while let Some(chunk) = body.frame().await {
///     //     match chunk {
///     //         Ok(frame) => {
///     //             if let Some(data) = frame.data_ref() {
///     //                 println!("Received chunk: {} bytes", data.len());
///     //             }
///     //         }
///     //         Err(e) => eprintln!("Error reading chunk: {}", e),
///     //     }
///     // }
/// }
/// ```
pub struct Bytes<'a>(pub &'a mut Incoming);

impl<'a> FromRequest<'a> for Bytes<'a> {
    type Error = Infallible;

    /// Extracts raw body access from an HTTP request.
    ///
    /// Returns a wrapper around the request's body stream, providing direct access
    /// to the underlying `hyper::body::Incoming` type for custom processing.
    ///
    /// This extractor never fails as it simply provides access to the existing
    /// body stream without performing any validation or processing.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, bytes::Bytes};
    /// use tako::types::Request;
    /// use http_body_util::BodyExt;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let Bytes(body) = Bytes::from_request(&mut req).await?;
    ///
    ///     // Use hyper's body utilities to read the body
    ///     let collected = body.collect().await?;
    ///     let bytes = collected.to_bytes();
    ///
    ///     println!("Request body size: {} bytes", bytes.len());
    ///
    ///     // Process the raw bytes as needed
    ///     if !bytes.is_empty() {
    ///         println!("First byte: 0x{:02x}", bytes[0]);
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Bytes(req.body_mut())))
    }
}
