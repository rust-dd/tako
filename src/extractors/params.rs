//! Path parameter extraction and deserialization for dynamic route segments.
//!
//! This module provides extractors for parsing path parameters from dynamic route segments
//! into strongly-typed Rust structures. It handles parameter extraction from routes like
//! `/users/{id}` or `/posts/{post_id}/comments/{comment_id}` and automatically deserializes
//! them using serde. The extractor supports type coercion for common types like integers,
//! floats, and strings, making it easy to work with typed path parameters in handlers.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::params::Params;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct UserParams {
//!     id: u64,
//!     name: String,
//! }
//!
//! // For route: /users/{id}/profile/{name}
//! async fn user_profile(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let params: Params<UserParams> = Params::from_request(&mut req).await?;
//!
//!     Ok(format!("User ID: {}, Name: {}", params.0.id, params.0.name))
//! }
//!
//! // Simple single parameter extraction
//! #[derive(Deserialize)]
//! struct IdParam {
//!     id: u32,
//! }
//!
//! async fn get_item(params: Params<IdParam>) -> String {
//!     format!("Item ID: {}", params.0.id)
//! }
//! ```

use std::{collections::HashMap, future::ready};

use http::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// Internal helper struct for storing path parameters extracted from routes.
///
/// `PathParams` is used internally by the Tako framework to store path parameters
/// extracted from dynamic route segments. It maintains a HashMap of parameter names
/// to their string values, which are later deserialized into strongly-typed structures
/// by the `Params` extractor.
///
/// # Examples
///
/// ```rust
/// use std::collections::HashMap;
/// use tako::extractors::params::PathParams;
///
/// let mut map = HashMap::new();
/// map.insert("id".to_string(), "123".to_string());
/// map.insert("category".to_string(), "electronics".to_string());
///
/// let path_params = PathParams(map);
/// assert_eq!(path_params.0.get("id"), Some(&"123".to_string()));
/// ```
#[derive(Clone, Default)]
pub(crate) struct PathParams(pub HashMap<String, String>);

/// Path parameter extractor with automatic deserialization to typed structures.
///
/// `Params<T>` extracts path parameters from dynamic route segments and deserializes
/// them into strongly-typed Rust structures using serde. It performs automatic type
/// coercion for common types (integers, floats, strings) and provides detailed error
/// information when deserialization fails.
///
/// # Type Parameters
///
/// * `T` - The target type for parameter deserialization, must implement `DeserializeOwned`
///
/// # Examples
///
/// ```rust
/// use tako::extractors::params::Params;
/// use serde::Deserialize;
///
/// // Simple parameter extraction
/// #[derive(Debug, Deserialize)]
/// struct PostParams {
///     post_id: u64,
///     slug: String,
/// }
///
/// // For route: /posts/{post_id}/{slug}
/// async fn show_post(params: Params<PostParams>) -> String {
///     format!("Post ID: {}, Slug: {}", params.0.post_id, params.0.slug)
/// }
///
/// // Multiple parameters with different types
/// #[derive(Deserialize)]
/// struct SearchParams {
///     category_id: u32,
///     page: Option<u32>,      // Optional parameters work too
///     sort: String,
/// }
///
/// // For route: /categories/{category_id}/items/{sort}?page={page}
/// async fn search_items(params: Params<SearchParams>) -> String {
///     let page = params.0.page.unwrap_or(1);
///     format!("Category: {}, Sort: {}, Page: {}",
///             params.0.category_id, params.0.sort, page)
/// }
/// ```
pub struct Params<T>(pub T);

/// Error types for path parameter extraction and deserialization.
///
/// These errors cover various failure modes when extracting and deserializing path
/// parameters, from missing parameter data to type conversion failures. Each error
/// provides specific information to help debug parameter parsing issues.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::params::{Params, ParamsError};
/// use tako::responder::Responder;
/// use http::StatusCode;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct BadParams {
///     id: u64,
///     invalid_field: String,
/// }
///
/// async fn handle_params_error(error: ParamsError) -> String {
///     match error {
///         ParamsError::MissingPathParams => "Route parameters not found".to_string(),
///         ParamsError::DeserializationError(msg) => format!("Parameter error: {}", msg),
///     }
/// }
/// ```
#[derive(Debug)]
pub enum ParamsError {
    /// Path parameters not found in request extensions (internal routing error).
    MissingPathParams,
    /// Parameter deserialization failed (type mismatch, missing field, etc.).
    DeserializationError(String),
}

impl Responder for ParamsError {
    /// Converts path parameter errors into appropriate HTTP error responses.
    ///
    /// Missing path parameters return 500 Internal Server Error (routing issue),
    /// while deserialization errors return 400 Bad Request with details about
    /// what went wrong during parameter parsing.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::params::ParamsError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = ParamsError::DeserializationError("Invalid ID format".to_string());
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
    fn into_response(self) -> crate::types::Response {
        match self {
            ParamsError::MissingPathParams => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Path parameters not found in request extensions",
            )
                .into_response(),
            ParamsError::DeserializationError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to deserialize path parameters: {}", err),
            )
                .into_response(),
        }
    }
}

impl<'a, T> FromRequest<'a> for Params<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Error = ParamsError;

    /// Extracts and deserializes path parameters from the HTTP request.
    ///
    /// This method retrieves path parameters stored in request extensions by the
    /// routing system, performs type coercion for common types, and deserializes
    /// the parameters into the target type using serde.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::params::Params;
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize)]
    /// struct ArticleParams {
    ///     article_id: u64,
    ///     section: String,
    ///     version: Option<u32>,
    /// }
    ///
    /// // For route: /articles/{article_id}/{section}
    /// async fn get_article(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let params: Params<ArticleParams> = Params::from_request(&mut req).await?;
    ///
    ///     let version = params.0.version.unwrap_or(1);
    ///     Ok(format!("Article {} in section '{}', version {}",
    ///                params.0.article_id, params.0.section, version))
    /// }
    ///
    /// // Numeric type coercion works automatically
    /// #[derive(Deserialize)]
    /// struct NumericParams {
    ///     user_id: u64,
    ///     score: f64,
    ///     count: i32,
    /// }
    ///
    /// async fn process_numbers(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let params: Params<NumericParams> = Params::from_request(&mut req).await?;
    ///
    ///     Ok(format!("User {}: score {:.2}, count {}",
    ///                params.0.user_id, params.0.score, params.0.count))
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_params(req))
    }
}

impl<T> Params<T>
where
    T: DeserializeOwned,
{
    /// Extracts and deserializes path parameters from the request.
    ///
    /// Retrieves path parameters from request extensions, applies type coercion
    /// for common numeric and string types, and deserializes them into the
    /// target type using serde's JSON deserialization.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::params::Params;
    /// use tako::types::Request;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct RouteParams {
    ///     id: u64,
    ///     name: String,
    /// }
    ///
    /// // This would typically be called by the FromRequest implementation
    /// fn extract_example(req: &Request) -> Result<Params<RouteParams>, tako::extractors::params::ParamsError> {
    ///     Params::extract_params(req)
    /// }
    /// ```
    fn extract_params(req: &Request) -> Result<Params<T>, ParamsError> {
        let path_params = req
            .extensions()
            .get::<PathParams>()
            .ok_or(ParamsError::MissingPathParams)?;

        let coerced = Self::coerce_params(&path_params.0);
        let value = Value::Object(coerced);
        let parsed = serde_json::from_value::<T>(value)
            .map_err(|e| ParamsError::DeserializationError(e.to_string()))?;

        Ok(Params(parsed))
    }

    /// Converts string parameters into JSON-compatible values with type coercion.
    ///
    /// This function attempts to intelligently convert string parameter values into
    /// appropriate JSON types by trying to parse them as integers, floats, or
    /// keeping them as strings. This enables automatic type conversion during
    /// serde deserialization.
    ///
    /// # Type Coercion Rules
    ///
    /// 1. Try parsing as signed integer (`i64`)
    /// 2. Try parsing as unsigned integer (`u64`)
    /// 3. Try parsing as floating point (`f64`)
    /// 4. Fall back to string value
    ///
    /// # Examples
    ///
    /// ```rust
    /// use std::collections::HashMap;
    /// use tako::extractors::params::Params;
    /// use serde_json::Value;
    ///
    /// let mut params = HashMap::new();
    /// params.insert("id".to_string(), "123".to_string());
    /// params.insert("score".to_string(), "98.5".to_string());
    /// params.insert("name".to_string(), "john".to_string());
    /// params.insert("active".to_string(), "true".to_string());
    ///
    /// let coerced = Params::<()>::coerce_params(&params);
    ///
    /// // Numbers are converted to JSON numbers
    /// assert!(matches!(coerced.get("id"), Some(Value::Number(_))));
    /// assert!(matches!(coerced.get("score"), Some(Value::Number(_))));
    ///
    /// // Non-numeric values remain as strings
    /// assert!(matches!(coerced.get("name"), Some(Value::String(_))));
    /// assert!(matches!(coerced.get("active"), Some(Value::String(_))));
    /// ```
    fn coerce_params(map: &HashMap<String, String>) -> Map<String, Value> {
        let mut result = Map::new();

        for (k, v) in map {
            let val = if let Ok(n) = v.parse::<i64>() {
                Value::Number(n.into())
            } else if let Ok(n) = v.parse::<u64>() {
                Value::Number(n.into())
            } else if let Ok(n) = v.parse::<f64>() {
                Value::Number(serde_json::Number::from_f64(n).unwrap_or_else(|| 0.into()))
            } else {
                Value::String(v.clone())
            };

            result.insert(k.clone(), val);
        }

        result
    }
}
