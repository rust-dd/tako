use bytes::Bytes;
use http::HeaderValue;
use http::StatusCode;
use serde::Serialize;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;
use tako_rs_core::extractors::is_json_content_type;
use tako_rs_core::extractors::json::JsonError;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Response;

/// A JSON body as a `T` that may borrow from the collected body.
pub struct JsonBorrowed<'a, T>(pub T, std::marker::PhantomData<&'a ()>);

impl<'a, T> FromRequest<'a> for JsonBorrowed<'a, T>
where
  T: serde::Deserialize<'a>,
{
  type Error = JsonError;

  fn from_request(
    req: &'a mut tako_rs_core::types::Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    use crate::zero_copy_extractors::bytes::CachedRequestBody;
    async move {
      if !is_json_content_type(req.headers()) {
        return Err(JsonError::InvalidContentType);
      }
      if req.extensions().get::<CachedRequestBody>().is_none() {
        let buf = collect_body(req).await.map_err(JsonError::BodyReadError)?;
        req.extensions_mut().insert(CachedRequestBody(buf));
      }

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
  fn into_response(self) -> tako_rs_core::types::Response {
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
        let mut res = Response::new(tako_rs_core::body::TakoBody::from(err.to_string()));
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
