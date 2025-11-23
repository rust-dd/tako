//! Raw request body access for HTTP requests.
//!
//! This module provides the [`Bytes`](crate::extractors::bytes::Bytes) extractor for accessing the raw HTTP request body
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

use std::convert::Infallible;

use crate::{body::TakoBody, extractors::FromRequest, types::Request};

/// Raw request body extractor that provides access to the underlying body stream.
///
/// This extractor wraps a reference to the raw request body implementing `http_body::Body`,
/// allowing direct access to the request body without buffering.
#[doc(alias = "bytes")]
pub struct Bytes<'a>(pub &'a mut TakoBody);

impl<'a> FromRequest<'a> for Bytes<'a> {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Ok(Bytes(req.body_mut())))
  }
}
