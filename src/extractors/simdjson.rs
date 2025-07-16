//! SIMD-accelerated JSON extraction from HTTP request bodies.
//!
//! This module provides the [`SimdJson`] extractor that leverages SIMD-accelerated JSON
//! parsing via the `simd_json` crate for high-performance deserialization of request bodies.
//! It offers similar functionality to standard JSON extractors but with potentially
//! better performance for large JSON payloads.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::simdjson::SimdJson;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Deserialize, Serialize)]
//! struct User {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(SimdJson(user): SimdJson<User>) -> SimdJson<User> {
//!     println!("Creating user: {}", user.name);
//!     // Process user creation...
//!     SimdJson(user)
//! }
//! ```

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

/// An extractor that (de)serializes JSON using SIMD-accelerated parsing.
///
/// `SimdJson<T>` behaves similarly to standard JSON extractors but leverages
/// SIMD-accelerated parsing for potentially higher performance, especially with
/// large JSON payloads. It automatically handles content-type validation,
/// request body reading, and deserialization.
///
/// The extractor also implements [`Responder`], allowing it to be returned
/// directly from handler functions for JSON responses.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::simdjson::SimdJson;
/// use serde::{Deserialize, Serialize};
///
/// #[derive(Deserialize, Serialize)]
/// struct ApiResponse {
///     success: bool,
///     message: String,
/// }
///
/// async fn api_handler(SimdJson(request): SimdJson<ApiResponse>) -> SimdJson<ApiResponse> {
///     // Process the request...
///     SimdJson(ApiResponse {
///         success: true,
///         message: "Request processed successfully".to_string(),
///     })
/// }
/// ```
pub struct SimdJson<T>(pub T);

/// Error type for SIMD JSON extraction.
///
/// Represents various failure modes that can occur when extracting and parsing
/// JSON data from HTTP request bodies using SIMD-accelerated parsing.
#[derive(Debug)]
pub enum SimdJsonError {
    /// Request content type is not recognized as JSON.
    InvalidContentType,
    /// Content-Type header is missing from the request.
    MissingContentType,
    /// Failed to read the request body.
    BodyReadError(String),
    /// Failed to deserialize JSON using SIMD parser.
    DeserializationError(String),
}

impl Responder for SimdJsonError {
    /// Converts the error into an HTTP response.
    ///
    /// Maps SIMD JSON extraction errors to appropriate HTTP status codes with
    /// descriptive error messages. All errors result in `400 Bad Request` as they
    /// indicate client-side issues with the request format or content.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::simdjson::SimdJsonError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = SimdJsonError::InvalidContentType;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    ///
    /// let error = SimdJsonError::MissingContentType;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
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
/// Accepts `application/json`, `application/*+json`, and other JSON-related
/// content types by checking both the main type/subtype and any suffix.
///
/// # Arguments
///
/// * `headers` - HTTP headers to examine for Content-Type
///
/// # Examples
///
/// ```rust
/// # use tako::extractors::simdjson::is_json_content_type;
/// use http::HeaderMap;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("content-type", "application/json".parse().unwrap());
/// # let result =
/// # // Assuming the function were public:
/// # headers.get("content-type").is_some();
/// # assert!(result);
///
/// headers.insert("content-type", "application/vnd.api+json".parse().unwrap());
/// # let result =
/// # // Would return true for JSON with suffix
/// # headers.get("content-type").is_some();
/// # assert!(result);
/// ```
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

impl<'a, T> FromRequest<'a> for SimdJson<T>
where
    T: DeserializeOwned + Send + 'static,
{
    type Error = SimdJsonError;

    /// Extracts SIMD JSON data from an HTTP request body.
    ///
    /// This implementation validates the content type, reads the request body,
    /// and deserializes the JSON using SIMD-accelerated parsing for potentially
    /// improved performance over standard JSON parsing.
    ///
    /// # Requirements
    ///
    /// - Content-Type must be recognized as JSON (e.g., `application/json`)
    /// - Request body must be valid JSON
    /// - JSON must be deserializable into type `T`
    ///
    /// # Errors
    ///
    /// Returns `SimdJsonError` if:
    /// - Content-Type header is missing or not JSON
    /// - Request body cannot be read
    /// - JSON parsing or deserialization fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, simdjson::SimdJson};
    /// use tako::types::Request;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct LoginRequest {
    ///     username: String,
    ///     password: String,
    /// }
    ///
    /// async fn login_handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let SimdJson(login) = SimdJson::<LoginRequest>::from_request(&mut req).await?;
    ///
    ///     println!("Login attempt for user: {}", login.username);
    ///     // Handle authentication...
    ///
    ///     Ok(())
    /// }
    /// ```
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

impl<T> Responder for SimdJson<T>
where
    T: Serialize,
{
    /// Converts the wrapped data into an HTTP JSON response.
    ///
    /// Serializes the contained data to JSON using SIMD-accelerated serialization
    /// and creates an HTTP response with appropriate headers. On success, returns
    /// a `200 OK` response with `application/json` content type.
    ///
    /// # Errors
    ///
    /// If serialization fails, returns a `500 Internal Server Error` response
    /// with the error message in plain text.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::simdjson::SimdJson;
    /// use tako::responder::Responder;
    /// use serde::Serialize;
    /// use http::StatusCode;
    ///
    /// #[derive(Serialize)]
    /// struct ApiResponse {
    ///     status: String,
    ///     data: Vec<i32>,
    /// }
    ///
    /// let response_data = ApiResponse {
    ///     status: "success".to_string(),
    ///     data: vec![1, 2, 3],
    /// };
    ///
    /// let json_response = SimdJson(response_data);
    /// let http_response = json_response.into_response();
    ///
    /// assert_eq!(http_response.status(), StatusCode::OK);
    /// assert_eq!(
    ///     http_response.headers().get("content-type").unwrap(),
    ///     "application/json"
    /// );
    /// ```
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
