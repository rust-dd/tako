//! Borrowed access to the unbuffered request body.
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

use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::types::Request;

/// The unbuffered request body. Not the `bytes::Bytes` buffer: import one under an alias,
/// such as `Bytes as BytesBody`, when a handler needs both.
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
