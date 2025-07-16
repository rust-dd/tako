//! JSON request body extraction and deserialization for API endpoints.
//!
//! This module provides extractors for parsing JSON request bodies into strongly-typed Rust
//! structures using serde. It validates Content-Type headers, reads request bodies efficiently,
//! and provides detailed error information for malformed JSON or incorrect content types.
//! The extractor integrates seamlessly with serde's derive macros for automatic JSON
//! deserialization of complex data structures.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::json::Json;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::{Deserialize, Serialize};
//!
//! #[derive(Debug, Deserialize, Serialize)]
//! struct CreateUser {
//!     name: String,
//!     email: String,
//!     age: u32,
//! }
//!
//! async fn create_user_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let user_data: Json<CreateUser> = Json::from_request(&mut req).await?;
//!
//!     // Access the deserialized data
//!     println!("Creating user: {} ({})", user_data.0.name, user_data.0.email);
//!
//!     Ok(format!("User {} created successfully", user_data.0.name))
//! }
//!
//! // Nested JSON structures work seamlessly
//! #[derive(Deserialize)]
//! struct ApiRequest {
//!     action: String,
//!     payload: serde_json::Value,
//!     metadata: Option<std::collections::HashMap<String, String>>,
//! }
//! ```

use http::StatusCode;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// JSON request body extractor with automatic deserialization.
///
/// `Json<T>` extracts and deserializes JSON request bodies into strongly-typed Rust
/// structures. It validates the Content-Type header, reads the entire request body,
/// and uses serde for deserialization. The generic type `T` must implement
/// `DeserializeOwned` to enable automatic JSON parsing.
///
/// # Type Parameters
///
/// * `T` - The target type for JSON deserialization, must implement `DeserializeOwned`
///
/// # Examples
///
/// ```rust
/// use tako::extractors::json::Json;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize)]
/// struct LoginRequest {
///     username: String,
///     password: String,
///     remember_me: Option<bool>,
/// }
///
/// async fn login_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
///     let login: Json<LoginRequest> = Json::from_request(&mut req).await?;
///
///     // Access deserialized fields
///     let remember = login.0.remember_me.unwrap_or(false);
///
///     if authenticate(&login.0.username, &login.0.password) {
///         Ok(format!("Welcome, {}! Remember me: {}", login.0.username, remember))
///     } else {
///         Ok("Invalid credentials".to_string())
///     }
/// }
///
/// fn authenticate(username: &str, password: &str) -> bool {
///     // Implement authentication logic
///     username == "admin" && password == "secret"
/// }
/// ```
pub struct Json<T>(pub T);

/// Error types for JSON extraction and deserialization.
///
/// These errors cover various failure modes when processing JSON request bodies,
/// from content type validation to serde deserialization errors. Each error
/// provides specific information to help debug JSON parsing issues.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::json::{Json, JsonError};
/// use tako::responder::Responder;
/// use http::StatusCode;
///
/// async fn handle_json_error(error: JsonError) -> String {
///     match error {
///         JsonError::InvalidContentType => "Please send JSON data".to_string(),
///         JsonError::DeserializationError(msg) => format!("JSON error: {}", msg),
///         _ => "Request processing error".to_string(),
///     }
/// }
/// ```
#[derive(Debug)]
pub enum JsonError {
    /// Content-Type header is not application/json or compatible JSON type.
    InvalidContentType,
    /// Content-Type header is missing from the request.
    MissingContentType,
    /// Failed to read the request body (network error, timeout, etc.).
    BodyReadError(String),
    /// JSON deserialization failed (syntax error, type mismatch, etc.).
    DeserializationError(String),
}

impl Responder for JsonError {
    /// Converts JSON extraction errors into appropriate HTTP error responses.
    ///
    /// Returns 400 Bad Request responses with descriptive error messages to help
    /// clients understand what went wrong with their JSON request.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::json::JsonError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = JsonError::InvalidContentType;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
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

/// Checks if the Content-Type header indicates JSON content.
///
/// Validates Content-Type headers against JSON media types including `application/json`
/// and `application/*+json` variants (e.g., `application/hal+json`). This follows
/// RFC standards for JSON content type detection.
///
/// # Examples
///
/// ```rust
/// use http::HeaderMap;
/// use tako::extractors::json::is_json_content_type;
///
/// let mut headers = HeaderMap::new();
/// headers.insert("content-type", "application/json".parse().unwrap());
/// assert!(is_json_content_type(&headers));
///
/// headers.insert("content-type", "application/hal+json".parse().unwrap());
/// assert!(is_json_content_type(&headers));
///
/// headers.insert("content-type", "text/plain".parse().unwrap());
/// assert!(!is_json_content_type(&headers));
/// ```
fn is_json_content_type(headers: &http::HeaderMap) -> bool {
    headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|ct| ct.parse::<mime_guess::Mime>().ok())
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

    /// Extracts and deserializes JSON data from the HTTP request body.
    ///
    /// This method performs the complete JSON extraction process:
    /// 1. Validates Content-Type header for JSON compatibility
    /// 2. Reads the entire request body into memory
    /// 3. Deserializes JSON using serde into the target type
    /// 4. Returns the wrapped deserialized data
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::json::Json;
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    /// use serde::{Deserialize, Serialize};
    ///
    /// #[derive(Debug, Deserialize, Serialize)]
    /// struct BlogPost {
    ///     title: String,
    ///     content: String,
    ///     tags: Vec<String>,
    ///     published: bool,
    /// }
    ///
    /// async fn create_post(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let post: Json<BlogPost> = Json::from_request(&mut req).await?;
    ///
    ///     // Process the deserialized blog post
    ///     let tag_count = post.0.tags.len();
    ///     let status = if post.0.published { "published" } else { "draft" };
    ///
    ///     Ok(format!("Created {} post '{}' with {} tags",
    ///                status, post.0.title, tag_count))
    /// }
    ///
    /// // Complex nested structures work automatically
    /// #[derive(Deserialize)]
    /// struct ApiResponse<T> {
    ///     success: bool,
    ///     data: Option<T>,
    ///     errors: Vec<String>,
    /// }
    ///
    /// async fn process_api_response(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let response: Json<ApiResponse<BlogPost>> = Json::from_request(&mut req).await?;
    ///
    ///     if response.0.success {
    ///         Ok("API call successful".to_string())
    ///     } else {
    ///         Ok(format!("API errors: {:?}", response.0.errors))
    ///     }
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        async move {
            // Validate Content-Type header for JSON compatibility
            if !is_json_content_type(req.headers()) {
                return Err(JsonError::InvalidContentType);
            }

            // Read the complete request body into memory
            let body_bytes = req
                .body_mut()
                .collect()
                .await
                .map_err(|e| JsonError::BodyReadError(e.to_string()))?
                .to_bytes();

            // Deserialize JSON using serde into the target type
            let data = serde_json::from_slice(&body_bytes)
                .map_err(|e| JsonError::DeserializationError(e.to_string()))?;

            Ok(Json(data))
        }
    }
}
