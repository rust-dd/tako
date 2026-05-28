use bytes::Bytes;
use http::HeaderValue;
use http::StatusCode;
use http_body_util::BodyExt;
use serde::Serialize;
use tako_core::body::TakoBody;
use tako_core::extractors::FromRequest;
use tako_core::extractors::is_json_content_type;
use tako_core::extractors::json::JsonError;
use tako_core::responder::Responder;
use tako_core::types::Response;

pub struct JsonBorrowed<'a, T>(pub T, std::marker::PhantomData<&'a ()>);

impl<'a, T> FromRequest<'a> for JsonBorrowed<'a, T>
where
  T: serde::Deserialize<'a>,
{
  type Error = JsonError;

  fn from_request(
    req: &'a mut tako_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      // EXT-6: match the owned `Json<T>` extractor and reject requests
      // that don't declare a JSON content-type. Without this guard a
      // client could submit `application/x-www-form-urlencoded` and we'd
      // run a generic deserializer over arbitrary bytes, conflicting
      // with sibling body extractors.
      if !is_json_content_type(req.headers()) {
        return Err(JsonError::InvalidContentType);
      }
      // Collect the body once and cache it in request extensions so the parsed
      // value can borrow from it for the lifetime of the request. Keyed by the
      // `CachedRequestBody` newtype to avoid colliding with other middleware
      // that might stash a raw `Bytes` value in extensions.
      use crate::zero_copy_extractors::bytes::CachedRequestBody;
      if req.extensions().get::<CachedRequestBody>().is_none() {
        let buf = req
          .body_mut()
          .collect()
          .await
          .map_err(|e| JsonError::BodyReadError(e.to_string()))?
          .to_bytes();
        req.extensions_mut().insert(CachedRequestBody(buf));
      }

      // SAFETY: We just inserted it above if it was missing.
      let body_bytes: &'a Bytes = &req
        .extensions()
        .get::<CachedRequestBody>()
        .expect("body bytes must be present in request extensions")
        .0;

      let value = serde_json::from_slice::<T>(body_bytes.as_ref())
        .map_err(|e| JsonError::DeserializationError(e.to_string()))?;

      Ok(JsonBorrowed(value, std::marker::PhantomData))
    }
  }
}

impl<T> Responder for JsonBorrowed<'_, T>
where
  T: Serialize,
{
  fn into_response(self) -> tako_core::types::Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => {
        let mut res = Response::new(tako_core::body::TakoBody::from(err.to_string()));
        *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
        );
        res
      }
    }
  }
}
