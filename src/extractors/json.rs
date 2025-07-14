/// This module provides the `Json` extractor, which is used to deserialize the body of a request into a strongly-typed JSON object.
use http::StatusCode;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// The `Json` struct is an extractor that wraps a deserialized JSON object of type `T`.
///
/// # Example
///
/// ```rust
/// use tako::extractors::json::Json;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct MyData {
///     field: String,
/// }
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let json_data: Json<MyData> = Json::from_request(&mut req).await?;
///     // Use the extracted JSON data here
///     Ok(())
/// }
/// ```
pub struct Json<T>(pub T);

/// Error type for JSON extraction.
#[derive(Debug)]
pub enum JsonError {
    InvalidContentType,
    MissingContentType,
    BodyReadError(String),
    DeserializationError(String),
}

impl Responder for JsonError {
    fn into_response(self) -> crate::types::Response {
        match self {
            JsonError::InvalidContentType => (
                StatusCode::BAD_REQUEST,
                "Invalid content type; expected application/json",
            )
                .into_response(),
            JsonError::MissingContentType => {
                (StatusCode::BAD_REQUEST, "Missing content type header").into_response()
            }
            JsonError::BodyReadError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to read request body: {}", err),
            )
                .into_response(),
            JsonError::DeserializationError(err) => (
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
fn is_json_content_type(headers: &http::HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| ct.parse::<mime::Mime>().ok())
        .map(|mime| {
            mime.type_() == "application"
                && (mime.subtype() == "json" || mime.suffix().is_some_and(|s| s == "json"))
        })
        .unwrap_or(false)
}

impl<'a, T> FromRequest<'a> for Json<T>
where
    T: DeserializeOwned + Send + 'static,
{
    type Error = JsonError;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        async move {
            // Check content type
            if !is_json_content_type(req.headers()) {
                return Err(JsonError::InvalidContentType);
            }

            // Read the request body
            let body_bytes = req
                .body_mut()
                .collect()
                .await
                .map_err(|e| JsonError::BodyReadError(e.to_string()))?
                .to_bytes();

            // Deserialize JSON
            let data = serde_json::from_slice(&body_bytes)
                .map_err(|e| JsonError::DeserializationError(e.to_string()))?;

            Ok(Json(data))
        }
    }
}
