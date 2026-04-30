use bytes::Bytes;
use http::HeaderValue;
use http::StatusCode;
use http_body_util::BodyExt;
use serde::Serialize;

use tako_core::body::TakoBody;
use tako_core::extractors::FromRequest;
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
      // Collect the body once and cache it in request extensions so the parsed
      // value can borrow from it for the lifetime of the request.
      if req.extensions().get::<Bytes>().is_none() {
        let buf = req
          .body_mut()
          .collect()
          .await
          .map_err(|e| JsonError::BodyReadError(e.to_string()))?
          .to_bytes();
        req.extensions_mut().insert(buf);
      }

      // SAFETY: We just inserted it above if it was missing.
      let body_bytes: &'a Bytes = req
        .extensions()
        .get::<Bytes>()
        .expect("body bytes must be present in request extensions");

      let value = serde_json::from_slice::<T>(body_bytes.as_ref())
        .map_err(|e| JsonError::DeserializationError(e.to_string()))?;

      Ok(JsonBorrowed(value, std::marker::PhantomData))
    }
  }
}

impl<'a, T> Responder for JsonBorrowed<'a, T>
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
