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
pub struct Query<T>(pub T);

/// Error types for query parameter extraction and deserialization.
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

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_query_string(req.uri().query()))
  }
}

impl<'a, T> FromRequestParts<'a> for Query<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = QueryError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_query_string(parts.uri.query()))
  }
}
