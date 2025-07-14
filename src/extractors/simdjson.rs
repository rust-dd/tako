use http::StatusCode;
use http_body_util::BodyExt;
use hyper::{
    HeaderMap,
    header::{self, HeaderValue},
};
use serde::{Serialize, de::DeserializeOwned};

use crate::{
    body::TakoBody,
    extractors::FromRequest,
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

/// Error type for SimdJson extraction.
#[derive(Debug)]
pub enum SimdJsonError {
    InvalidContentType,
    MissingContentType,
    BodyReadError(String),
    DeserializationError(String),
}

impl Responder for SimdJsonError {
    fn into_response(self) -> Response {
        match self {
            SimdJsonError::InvalidContentType => (
                StatusCode::BAD_REQUEST,
                "Invalid content type; expected JSON",
            )
                .into_response(),
            SimdJsonError::MissingContentType => {
                (StatusCode::BAD_REQUEST, "Missing content type header").into_response()
            }
            SimdJsonError::BodyReadError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to read request body: {}", err),
            )
                .into_response(),
            SimdJsonError::DeserializationError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to deserialize JSON: {}", err),
            )
                .into_response(),
        }
    }
}

/// Returns `true` when the `Content-Type` header denotes JSON.
///
/// Accepts `application/json`, `application/*+json`, etc.
fn is_json_content_type(headers: &HeaderMap) -> bool {
    headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| ct.parse::<mime_guess::Mime>().ok())
        .map(|mime| {
            mime.type_() == "application"
                && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
        })
        .unwrap_or(false)
}

/// Implementation of [`FromRequest`] for [`SimdJson`].
///
/// Enables asynchronous extraction of JSON payloads from HTTP requests while
/// leveraging SIMD-accelerated parsing via the `simd_json` crate.
impl<'a, T> FromRequest<'a> for SimdJson<T>
where
    T: DeserializeOwned + Send + 'static,
{
    type Error = SimdJsonError;

    /// Attempts to construct a `SimdJson<T>` from the incoming HTTP request.
    ///
    /// The extraction fails when:
    /// 1. The `Content-Type` header is not recognised as JSON.
    /// 2. The request body cannot be fully read.
    /// 3. Deserialisation with `simd_json` fails.
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        async move {
            // Basic content-type validation so we can fail fast.
            if !is_json_content_type(req.headers()) {
                return Err(SimdJsonError::InvalidContentType);
            }

            // Collect the entire request body.
            let bytes = req
                .body_mut()
                .collect()
                .await
                .map_err(|e| SimdJsonError::BodyReadError(e.to_string()))?
                .to_bytes();

            let mut owned = bytes.to_vec();

            // SIMD-accelerated deserialization.
            let data = simd_json::from_slice::<T>(&mut owned)
                .map_err(|e| SimdJsonError::DeserializationError(e.to_string()))?;

            Ok(SimdJson(data))
        }
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
