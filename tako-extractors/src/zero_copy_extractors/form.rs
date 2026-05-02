//! Zero-copy form-urlencoded extractor.
//!
//! Mirrors `JsonBorrowed`: the body is collected once and cached in request
//! extensions, then deserialized via `serde_urlencoded::from_bytes` into a `T`
//! that borrows from the cached buffer.

use bytes::Bytes;
use http::StatusCode;
use http_body_util::BodyExt;
use tako_core::extractors::FromRequest;
use tako_core::responder::Responder;

/// Zero-copy `application/x-www-form-urlencoded` extractor.
///
/// The deserialized value `T` may borrow string slices from the cached body
/// buffer, so callers can decode forms without per-field allocations.
pub struct FormBorrowed<'a, T>(pub T, std::marker::PhantomData<&'a ()>);

/// Error type for `FormBorrowed`.
#[derive(Debug)]
pub enum FormBorrowedError {
  /// Content-Type is not `application/x-www-form-urlencoded`.
  InvalidContentType,
  /// Body collection failed.
  BodyReadError(String),
  /// `serde_urlencoded` failed to deserialize.
  DeserializationError(String),
}

impl Responder for FormBorrowedError {
  fn into_response(self) -> tako_core::types::Response {
    match self {
      Self::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "invalid content type; expected application/x-www-form-urlencoded",
      )
        .into_response(),
      Self::BodyReadError(e) => (
        StatusCode::BAD_REQUEST,
        format!("failed to read request body: {e}"),
      )
        .into_response(),
      Self::DeserializationError(e) => (
        StatusCode::BAD_REQUEST,
        format!("failed to deserialize form: {e}"),
      )
        .into_response(),
    }
  }
}

impl<'a, T> FromRequest<'a> for FormBorrowed<'a, T>
where
  T: serde::Deserialize<'a>,
{
  type Error = FormBorrowedError;

  fn from_request(
    req: &'a mut tako_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let ct = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
      if !ct.starts_with("application/x-www-form-urlencoded") {
        return Err(FormBorrowedError::InvalidContentType);
      }

      if req.extensions().get::<Bytes>().is_none() {
        let buf = req
          .body_mut()
          .collect()
          .await
          .map_err(|e| FormBorrowedError::BodyReadError(e.to_string()))?
          .to_bytes();
        req.extensions_mut().insert(buf);
      }

      let body_bytes: &'a Bytes = req
        .extensions()
        .get::<Bytes>()
        .expect("body bytes must be present in request extensions");

      let value: T = serde_urlencoded::from_bytes(body_bytes.as_ref())
        .map_err(|e| FormBorrowedError::DeserializationError(e.to_string()))?;

      Ok(FormBorrowed(value, std::marker::PhantomData))
    }
  }
}
