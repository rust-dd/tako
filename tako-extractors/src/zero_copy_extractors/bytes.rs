//! Zero-copy body extractor.
//!
//! `BytesBorrowed<'a>` collects the request body into request extensions on
//! first access and hands subsequent extractors a borrowed `&'a Bytes` so the
//! same body can drive several zero-copy parses (form, json, custom) without
//! re-collecting or cloning.

use bytes::Bytes;
use http_body_util::BodyExt;
use tako_core::extractors::FromRequest;

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

/// Error returned while collecting the request body.
#[derive(Debug)]
pub struct BytesReadError(pub String);

impl tako_core::responder::Responder for BytesReadError {
  fn into_response(self) -> tako_core::types::Response {
    (
      http::StatusCode::BAD_REQUEST,
      format!("failed to read request body: {}", self.0),
    )
      .into_response()
  }
}

impl<'a> FromRequest<'a> for BytesBorrowed<'a> {
  type Error = BytesReadError;

  fn from_request(
    req: &'a mut tako_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.extensions().get::<CachedRequestBody>().is_none() {
        let buf = req
          .body_mut()
          .collect()
          .await
          .map_err(|e| BytesReadError(e.to_string()))?
          .to_bytes();
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
  /// [`Infallible`] and swallowed a body-read failure by caching an empty
  /// slice, which made downstream parsers report "empty body" for what was
  /// really a transport-level error. Propagate the underlying read failure
  /// so the caller can distinguish the two.
  type Error = BytesReadError;

  fn from_request(
    req: &'a mut tako_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if req.extensions().get::<CachedRequestBody>().is_none() {
        let collected = req
          .body_mut()
          .collect()
          .await
          .map_err(|e| BytesReadError(e.to_string()))?;
        req
          .extensions_mut()
          .insert(CachedRequestBody(collected.to_bytes()));
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
