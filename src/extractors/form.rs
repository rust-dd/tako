use http::StatusCode;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// Represents a form extracted from an HTTP request body.
///
/// This generic struct wraps the deserialized form data of type `T`.
pub struct Form<T>(pub T);

/// Error type for Form extraction.
#[derive(Debug)]
pub enum FormError {
    InvalidContentType,
    BodyReadError(String),
    InvalidUtf8,
    ParseError(String),
    DeserializationError(String),
}

impl Responder for FormError {
    fn into_response(self) -> crate::types::Response {
        match self {
            FormError::InvalidContentType => (
                StatusCode::BAD_REQUEST,
                "Invalid content type; expected application/x-www-form-urlencoded",
            )
                .into_response(),
            FormError::BodyReadError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to read request body: {}", err),
            )
                .into_response(),
            FormError::InvalidUtf8 => (
                StatusCode::BAD_REQUEST,
                "Request body contains invalid UTF-8",
            )
                .into_response(),
            FormError::ParseError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to parse form data: {}", err),
            )
                .into_response(),
            FormError::DeserializationError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to deserialize form data: {}", err),
            )
                .into_response(),
        }
    }
}

impl<'a, T> FromRequest<'a> for Form<T>
where
    T: DeserializeOwned + Send + 'static,
{
    type Error = FormError;

    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        async move {
            // Check content type
            let content_type = req
                .headers()
                .get(hyper::header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok());

            if content_type != Some("application/x-www-form-urlencoded") {
                return Err(FormError::InvalidContentType);
            }

            // Read the request body
            let body_bytes = req
                .body_mut()
                .collect()
                .await
                .map_err(|e| FormError::BodyReadError(e.to_string()))?
                .to_bytes();

            // Convert to string
            let body_str = std::str::from_utf8(&body_bytes).map_err(|_| FormError::InvalidUtf8)?;

            // Parse form data
            let form_data = url::form_urlencoded::parse(body_str.as_bytes())
                .into_owned()
                .collect::<Vec<(String, String)>>();

            // Convert to HashMap
            let form_map = HashMap::<String, String>::from_iter(form_data);

            // Convert to JSON value for deserialization
            let json_value =
                serde_json::to_value(form_map).map_err(|e| FormError::ParseError(e.to_string()))?;

            // Deserialize to target type
            let form_data = serde_json::from_value::<T>(json_value)
                .map_err(|e| FormError::DeserializationError(e.to_string()))?;

            Ok(Form(form_data))
        }
    }
}
