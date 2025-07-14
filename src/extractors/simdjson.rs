use anyhow::{Result, anyhow};
use http::StatusCode;
use http_body_util::BodyExt;
use hyper::{
    HeaderMap,
    header::{self, HeaderValue},
};
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::{
    body::TakoBody,
    extractors::AsyncFromRequestMut,
    responder::Responder,
    types::{Request, Response},
};

/// An extractor that (de)serializes JSON using the [`simd_json`] crate.
///
/// `SimdJson<T>` behaves similarly to the built-in `Json` extractor but leverages
/// SIMD-accelerated parsing for higher performance.
///
/// # Example
///
/// ```rust
/// use tako::extractors::simdjson::SimdJson;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Serialize, Deserialize)]
/// struct Payload {
///     name: String,
/// }
///
/// async fn handler(mut req: tako::types::Request) -> anyhow::Result<SimdJson<Payload>> {
///     SimdJson::<Payload>::from_request(&mut req).await
/// }
/// ```
pub struct SimdJson<T>(pub T);

/// Returns `true` when the `Content-Type` header denotes JSON.
///
/// Accepts `application/json`, `application/*+json`, etc.
fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| ct.parse::<mime::Mime>().ok())
        .map(|mime| {
            mime.type_() == "application"
                && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
        })
        .unwrap_or(false)
}

/// Implementation of [`AsyncFromRequestMut`] for [`SimdJson`].
///
/// Enables asynchronous extraction of JSON payloads from HTTP requests while
/// leveraging SIMD-accelerated parsing via the `simd_json` crate.
impl<'a, T> AsyncFromRequestMut<'a> for SimdJson<T>
where
    T: DeserializeOwned + Send + 'static,
{
    /// Attempts to construct a `SimdJson<T>` from the incoming HTTP request.
    ///
    /// The extraction fails when:
    /// 1. The `Content-Type` header is not recognised as JSON.
    /// 2. The request body cannot be fully read.
    /// 3. Deserialisation with `simd_json` fails.
    async fn from_request(req: &'a mut Request) -> Result<Self> {
        // Basic content-type validation so we can fail fast.
        if !is_json_content_type(req.headers()) {
            return Err(anyhow!("invalid content type; expected JSON"));
        }

        // Collect the entire request body.
        let bytes = req.body_mut().collect().await?.to_bytes();
        let mut owned = bytes.to_vec();

        // SIMD-accelerated deserialization.
        let data = simd_json::from_slice::<T>(&mut owned)?;
        Ok(SimdJson(data))
    }
}

/// Implementation of [`Responder`] for [`SimdJson`].
///
/// Allows returning `SimdJson<T>` directly from handler functions, letting the
/// framework handle JSON serialisation and response construction.
impl<T> Responder for SimdJson<T>
where
    T: Serialize,
{
    /// Converts the wrapped data into an HTTP response.
    ///
    /// On success this method returns a `200 OK` response with a JSON body. If
    /// serialisation fails, a `500 Internal Server Error` is returned containing
    /// the error message in plain-text form.
    fn into_response(self) -> Response {
        match simd_json::to_vec(&self.0) {
            Ok(buf) => {
                let mut res = Response::new(TakoBody::from(buf));
                res.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
                );
                res
            }
            Err(err) => {
                let mut res = Response::new(TakoBody::from(err.to_string()));
                *res.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
                res.headers_mut().insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static(mime::TEXT_PLAIN_UTF_8.as_ref()),
                );
                res
            }
        }
    }
}
