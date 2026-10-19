//! The collected request body, borrowed as `&Bytes` or `&[u8]`.

use bytes::Bytes;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;

/// The collected body in the request extensions; a newtype so a raw `Bytes` from other code
/// cannot collide with it.
#[derive(Clone)]
pub struct CachedRequestBody(pub Bytes);

/// The body, collected into [`CachedRequestBody`] by the first extractor that needs it.
pub struct BytesBorrowed<'a>(pub &'a Bytes);

pub use tako_rs_core::extractors::body::BodyReadError as BytesReadError;

impl<'a> FromRequest<'a> for BytesBorrowed<'a> {
  type Error = BytesReadError;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.extensions().get::<CachedRequestBody>().is_none() {
        let buf = collect_body(req).await?;
        req.extensions_mut().insert(CachedRequestBody(buf));
      }

      let body_bytes: &'a Bytes = &req
        .extensions()
        .get::<CachedRequestBody>()
        .expect("body bytes must be present in request extensions")
        .0;

      Ok(BytesBorrowed(body_bytes))
    }
  }
}

/// [`BytesBorrowed`] as a slice.
pub struct BodySliceBorrowed<'a>(pub &'a [u8]);

impl<'a> FromRequest<'a> for BodySliceBorrowed<'a> {
  type Error = BytesReadError;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.extensions().get::<CachedRequestBody>().is_none() {
        let collected = collect_body(req).await?;
        req.extensions_mut().insert(CachedRequestBody(collected));
      }

      let bytes: &'a Bytes = &req
        .extensions()
        .get::<CachedRequestBody>()
        .expect("body bytes must be present in request extensions")
        .0;

      Ok(BodySliceBorrowed(bytes.as_ref()))
    }
  }
}
