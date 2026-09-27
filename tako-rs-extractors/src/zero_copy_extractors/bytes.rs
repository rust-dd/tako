//! Zero-copy body extractor.
//!
//! `BytesBorrowed<'a>` collects the request body into request extensions on
//! first access and hands subsequent extractors a borrowed `&'a Bytes` so the
//! same body can drive several zero-copy parses (form, json, custom) without
//! re-collecting or cloning.

use bytes::Bytes;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;

/// Wrapper around the cached request body inserted into request extensions.
///
/// Using a newtype prevents collisions with other middleware that might also
/// stash a raw [`Bytes`] in extensions for unrelated purposes — both inserts
/// would otherwise share the same `TypeId` and clobber each other.
#[derive(Clone)]
pub struct CachedRequestBody(pub Bytes);

/// Zero-copy access to the cached request body bytes.
///
/// On first call the body is collected and stored in request extensions; later
/// calls return the cached reference.
pub struct BytesBorrowed<'a>(pub &'a Bytes);

/// Error returned while buffering the request body.
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

/// Zero-copy convenience that yields the cached body as `&'a [u8]`.
pub struct BodySliceBorrowed<'a>(pub &'a [u8]);

impl<'a> FromRequest<'a> for BodySliceBorrowed<'a> {
  /// Mirrors [`BytesBorrowed::Error`]. Previously this extractor returned
  /// [`std::convert::Infallible`] and swallowed a body-read failure by caching
  /// an empty slice, which made downstream parsers report "empty body" for
  /// what was really a transport-level error. Propagate the underlying read
  /// failure so the caller can distinguish the two.
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
