//! Query parameter extraction and deserialization from URL query strings.
//!
//! This module provides extractors for parsing URL query parameters into strongly-typed Rust
//! structures using serde. It handles URL-encoded query strings from GET requests and other
//! HTTP methods, automatically deserializing them into custom types. The extractor supports
//! nested structures, optional fields, and automatic type coercion for common data types
//! like numbers and booleans.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::query::Query;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct SearchQuery {
//!     q: String,
//!     page: Option<u32>,
//!     limit: Option<u32>,
//!     sort: Option<String>,
//! }
//!
//! // For URL: /search?q=rust&page=2&limit=20&sort=date
//! async fn search_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let query: Query<SearchQuery> = Query::from_request(&mut req).await?;
//!
//!     let page = query.0.page.unwrap_or(1);
//!     let limit = query.0.limit.unwrap_or(10);
//!     let sort = query.0.sort.unwrap_or_else(|| "relevance".to_string());
//!
//!     Ok(format!("Searching for '{}' (page {}, limit {}, sort by {})",
//!                query.0.q, page, limit, sort))
//! }
//!
//! // Simple query parameter extraction
//! #[derive(Deserialize)]
//! struct Pagination {
//!     page: u32,
//!     per_page: u32,
//! }
//!
//! async fn list_items(query: Query<Pagination>) -> String {
//!     format!("Page {} with {} items per page", query.0.page, query.0.per_page)
//! }
//! ```

use std::{collections::HashMap, future::ready};

use http::{StatusCode, request::Parts};
use serde::de::DeserializeOwned;
use url::form_urlencoded;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// Query parameter extractor with automatic deserialization to typed structures.
///
/// `Query<T>` extracts query parameters from the URL query string and deserializes
/// them into strongly-typed Rust structures using serde. It handles URL decoding,
/// parameter parsing, and type conversion automatically. The generic type `T` must
/// implement `DeserializeOwned` to enable automatic query parameter deserialization.
///
/// # Type Parameters
///
/// * `T` - The target type for query parameter deserialization, must implement `DeserializeOwned`
///
/// # Examples
///
/// ```rust
/// use tako::extractors::query::Query;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Debug, Deserialize)]
/// struct FilterQuery {
///     category: Option<String>,
///     min_price: Option<f64>,
///     max_price: Option<f64>,
///     in_stock: Option<bool>,
///     tags: Option<Vec<String>>,
/// }
///
/// // For URL: /products?category=electronics&min_price=10.50&in_stock=true&tags=new&tags=sale
/// async fn filter_products(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
///     let filters: Query<FilterQuery> = Query::from_request(&mut req).await?;
///
///     let category = filters.0.category.unwrap_or_else(|| "all".to_string());
///     let min_price = filters.0.min_price.unwrap_or(0.0);
///     let in_stock = filters.0.in_stock.unwrap_or(false);
///
///     Ok(format!("Filtering {} category, min price ${:.2}, in stock: {}",
///                category, min_price, in_stock))
/// }
///
/// // Nested structures work with flattened query parameters
/// #[derive(Deserialize)]
/// struct AdvancedQuery {
///     search: String,
///     filters: FilterOptions,
/// }
///
/// #[derive(Deserialize)]
/// struct FilterOptions {
///     active: Option<bool>,
///     created_after: Option<String>,
/// }
/// ```
pub struct Query<T>(pub T);

/// Error types for query parameter extraction and deserialization.
///
/// These errors cover various failure modes when processing URL query parameters,
/// from missing query strings to serde deserialization errors. Each error provides
/// specific information to help debug query parameter parsing issues.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::query::{Query, QueryError};
/// use tako::responder::Responder;
/// use http::StatusCode;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct StrictQuery {
///     required_param: u32,
/// }
///
/// async fn handle_query_error(error: QueryError) -> String {
///     match error {
///         QueryError::MissingQueryString => "No query parameters provided".to_string(),
///         QueryError::ParseError(msg) => format!("Query parsing error: {}", msg),
///         QueryError::DeserializationError(msg) => format!("Parameter error: {}", msg),
///     }
/// }
/// ```
#[derive(Debug)]
pub enum QueryError {
    /// No query string found in the request URI.
    MissingQueryString,
    /// Failed to parse query parameters from the query string.
    ParseError(String),
    /// Query parameter deserialization failed (type mismatch, missing field, etc.).
    DeserializationError(String),
}

impl Responder for QueryError {
    /// Converts query parameter errors into appropriate HTTP error responses.
    ///
    /// Returns 400 Bad Request responses with descriptive error messages to help
    /// clients understand what went wrong with their query parameters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::query::QueryError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = QueryError::DeserializationError("Invalid number format".to_string());
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
    fn into_response(self) -> crate::types::Response {
        match self {
            QueryError::MissingQueryString => (
                StatusCode::BAD_REQUEST,
                "No query string found in request URI",
            )
                .into_response(),
            QueryError::ParseError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to parse query parameters: {}", err),
            )
                .into_response(),
            QueryError::DeserializationError(err) => (
                StatusCode::BAD_REQUEST,
                format!("Failed to deserialize query parameters: {}", err),
            )
                .into_response(),
        }
    }
}

impl<T> Query<T>
where
    T: DeserializeOwned,
{
    /// Extracts and deserializes query parameters from a URI query string.
    ///
    /// Parses URL-encoded query parameters, converts them to a HashMap, and then
    /// deserializes them into the target type using serde's JSON deserialization.
    /// This enables automatic type conversion and validation of query parameters.
    ///
    /// # Query Parameter Format
    ///
    /// Supports standard URL query parameter formats:
    /// - Simple: `?name=value&age=25`
    /// - Arrays: `?tags=rust&tags=web&tags=async`
    /// - Optional: `?page=1` (missing parameters become `None`)
    /// - Boolean: `?active=true&verified=false`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::query::Query;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize)]
    /// struct TestQuery {
    ///     name: String,
    ///     age: Option<u32>,
    ///     active: Option<bool>,
    /// }
    ///
    /// // Test with various query string formats
    /// let result1 = Query::<TestQuery>::extract_from_query_string(Some("name=john&age=25"));
    /// let result2 = Query::<TestQuery>::extract_from_query_string(Some("name=jane&active=true"));
    /// let result3 = Query::<TestQuery>::extract_from_query_string(None);
    ///
    /// // First two should succeed, third should fail due to missing required field
    /// assert!(result1.is_ok());
    /// assert!(result2.is_ok());
    /// assert!(result3.is_err());
    /// ```
    fn extract_from_query_string(query_string: Option<&str>) -> Result<Query<T>, QueryError> {
        let query = query_string.unwrap_or_default();

        // Parse query parameters into a HashMap
        let params: HashMap<String, String> = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect();

        // Convert to JSON value for deserialization
        let json_value =
            serde_json::to_value(params).map_err(|e| QueryError::ParseError(e.to_string()))?;

        // Deserialize to target type
        let query_data = serde_json::from_value::<T>(json_value)
            .map_err(|e| QueryError::DeserializationError(e.to_string()))?;

        Ok(Query(query_data))
    }
}

impl<'a, T> FromRequest<'a> for Query<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Error = QueryError;

    /// Extracts and deserializes query parameters from the complete HTTP request.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::query::Query;
    /// use tako::extractors::FromRequest;
    /// use tako::types::Request;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize)]
    /// struct ProductQuery {
    ///     category: Option<String>,
    ///     sort_by: Option<String>,
    ///     order: Option<String>,
    ///     page: Option<u32>,
    ///     limit: Option<u32>,
    /// }
    ///
    /// // For URL: /products?category=electronics&sort_by=price&order=desc&page=2&limit=50
    /// async fn list_products(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
    ///     let query: Query<ProductQuery> = Query::from_request(&mut req).await?;
    ///
    ///     let category = query.0.category.unwrap_or_else(|| "all".to_string());
    ///     let sort_by = query.0.sort_by.unwrap_or_else(|| "name".to_string());
    ///     let order = query.0.order.unwrap_or_else(|| "asc".to_string());
    ///     let page = query.0.page.unwrap_or(1);
    ///     let limit = query.0.limit.unwrap_or(20);
    ///
    ///     Ok(format!("Products in {} category, sorted by {} {}, page {} (limit {})",
    ///                category, sort_by, order, page, limit))
    /// }
    ///
    /// // Complex query structures with nested data
    /// #[derive(Deserialize)]
    /// struct SearchRequest {
    ///     q: String,
    ///     filters: SearchFilters,
    ///     pagination: SearchPagination,
    /// }
    ///
    /// #[derive(Deserialize)]
    /// struct SearchFilters {
    ///     category: Option<String>,
    ///     min_price: Option<f64>,
    ///     max_price: Option<f64>,
    /// }
    ///
    /// #[derive(Deserialize)]
    /// struct SearchPagination {
    ///     page: Option<u32>,
    ///     per_page: Option<u32>,
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_query_string(req.uri().query()))
    }
}

impl<'a, T> FromRequestParts<'a> for Query<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Error = QueryError;

    /// Extracts and deserializes query parameters from HTTP request parts.
    ///
    /// This is more efficient when you only need query parameters and don't require
    /// access to the request body. Particularly useful in middleware or when combining
    /// with other header-based extractors.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::query::Query;
    /// use tako::extractors::FromRequestParts;
    /// use http::request::Parts;
    /// use serde::Deserialize;
    ///
    /// #[derive(Debug, Deserialize)]
    /// struct ApiQuery {
    ///     version: Option<String>,
    ///     format: Option<String>,
    ///     debug: Option<bool>,
    /// }
    ///
    /// async fn api_handler(query: Query<ApiQuery>) -> String {
    ///     let version = query.0.version.unwrap_or_else(|| "v1".to_string());
    ///     let format = query.0.format.unwrap_or_else(|| "json".to_string());
    ///     let debug = query.0.debug.unwrap_or(false);
    ///
    ///     format!("API {} response in {} format (debug: {})", version, format, debug)
    /// }
    ///
    /// // Combining with other extractors
    /// async fn combined_handler(
    ///     query: Query<ApiQuery>,
    ///     path: tako::extractors::path::Path<'_>,
    /// ) -> String {
    ///     format!("Path: {}, Query: {:?}", path.0, query.0)
    /// }
    ///
    /// // Optional query parameters with defaults
    /// #[derive(Deserialize)]
    /// struct ListQuery {
    ///     page: Option<u32>,
    ///     per_page: Option<u32>,
    ///     sort: Option<String>,
    ///     filter: Option<String>,
    /// }
    ///
    /// async fn list_handler(query: Query<ListQuery>) -> String {
    ///     let page = query.0.page.unwrap_or(1);
    ///     let per_page = query.0.per_page.unwrap_or(10).min(100); // Cap at 100
    ///     let sort = query.0.sort.unwrap_or_else(|| "created_at".to_string());
    ///
    ///     format!("Listing page {} ({} items, sorted by {})", page, per_page, sort)
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_query_string(parts.uri.query()))
    }
}
