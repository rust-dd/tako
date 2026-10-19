//! A form body deserialized into a `T` that borrows from the collected body.

use bytes::Bytes;
use http::StatusCode;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;
use tako_rs_core::responder::Responder;

/// An `application/x-www-form-urlencoded` body as a `T` that may borrow from the collected body.
pub struct FormBorrowed<'a, T>(pub T, std::marker::PhantomData<&'a ()>);

/// Rejection of [`FormBorrowed`].
#[derive(Debug)]
pub enum FormBorrowedError {
  InvalidContentType,
  BodyReadError(tako_rs_core::extractors::body::BodyReadError),
  DeserializationError(String),
}

impl Responder for FormBorrowedError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      Self::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "invalid content type; expected application/x-www-form-urlencoded",
      )
        .into_response(),
      Self::BodyReadError(e) => {
        (e.status(), format!("failed to read request body: {e}")).into_response()
      }
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
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      use crate::zero_copy_extractors::bytes::CachedRequestBody;

      let ct = req
        .headers()
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
      if !ct.starts_with("application/x-www-form-urlencoded") {
        return Err(FormBorrowedError::InvalidContentType);
      }

      if req.extensions().get::<CachedRequestBody>().is_none() {
        let buf = collect_body(req)
          .await
          .map_err(FormBorrowedError::BodyReadError)?;
        req.extensions_mut().insert(CachedRequestBody(buf));
      }

      let body_bytes: &'a Bytes = &req
        .extensions()
        .get::<CachedRequestBody>()
        .expect("body bytes must be present in request extensions")
        .0;

      let value: T = serde_urlencoded::from_bytes(body_bytes.as_ref())
        .map_err(|e| FormBorrowedError::DeserializationError(e.to_string()))?;

      Ok(FormBorrowed(value, std::marker::PhantomData))
    }
  }
}
