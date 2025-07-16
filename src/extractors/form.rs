//! Form data extraction from HTTP request bodies.
//!
//! This module provides the [`Form`] extractor for parsing `application/x-www-form-urlencoded`
//! request bodies into strongly-typed Rust structures. It uses serde for deserialization,
//! allowing automatic parsing of form data into any type that implements `DeserializeOwned`.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::form::Form;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct LoginForm {
//!     username: String,
//!     password: String,
//! }
//!
//! async fn login_handler(Form(form): Form<LoginForm>) {
//!     println!("Username: {}", form.username);
//!     // Handle login logic...
//! }
//! ```

use http::StatusCode;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;
use std::collections::HashMap;

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// Represents a form extracted from an HTTP request body.
///
/// This generic struct wraps the deserialized form data of type `T`. It automatically
/// parses `application/x-www-form-urlencoded` request bodies and deserializes them
/// into the specified type using serde.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::form::Form;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct ContactForm {
///     name: String,
///     email: String,
///     message: String,
/// }
///
/// async fn contact_handler(Form(contact): Form<ContactForm>) {
///     println!("Received message from {} ({}): {}",
///              contact.name, contact.email, contact.message);
/// }
/// ```
pub struct Form<T>(pub T);

/// Error type for Form extraction.
///
/// Represents various failure modes that can occur when extracting and parsing
/// form data from HTTP request bodies.
#[derive(Debug)]
pub enum FormError {
    /// Request content type is not `application/x-www-form-urlencoded`.
    InvalidContentType,
    /// Failed to read the request body.
    BodyReadError(String),
    /// Request body contains invalid UTF-8 sequences.
    InvalidUtf8,
    /// Failed to parse the form data format.
    ParseError(String),
    /// Failed to deserialize form data into the target type.
    DeserializationError(String),
}

impl Responder for FormError {
    /// Converts the error into an HTTP response.
    ///
    /// Maps form extraction errors to appropriate HTTP status codes with descriptive
    /// error messages. All errors result in `400 Bad Request` as they indicate
    /// client-side issues with the request format or content.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::form::FormError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = FormError::InvalidContentType;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    ///
    /// let error = FormError::InvalidUtf8;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
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
