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
/// It's useful for scenarios where you need streaming request body processing or custom parsing logic.
pub struct Bytes<'a>(pub &'a mut Incoming);

impl<'a> FromRequest<'a> for Bytes<'a> {
    type Error = Infallible;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Ok(Bytes(req.body_mut())))
    }
}
