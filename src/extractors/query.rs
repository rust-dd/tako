/// This module provides the `Query` extractor, which is used to extract query parameters from a request.
///
/// The `Query` extractor allows deserialization of query parameters into a strongly-typed structure,
/// making it easier to work with query strings in a type-safe manner.
use std::{collections::HashMap, future::ready};

use http::{StatusCode, request::Parts};
use serde::de::DeserializeOwned;
use url::form_urlencoded;

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

/// The `Query` struct is an extractor that wraps a deserialized representation of the query parameters.
///
/// # Example
///
/// ```rust
/// use tako::extractors::query::Query;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct MyQuery {
///     param1: String,
///     param2: i32,
/// }
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let query = Query::<MyQuery>::from_request(&mut req).await?;
///     // Use the extracted query parameters here
///     Ok(())
/// }
/// ```
pub struct Query<T>(pub T);

/// Error type for query parameter extraction.
#[derive(Debug)]
pub enum QueryError {
    MissingQueryString,
    ParseError(String),
    DeserializationError(String),
}

impl Responder for QueryError {
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

    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_query_string(parts.uri.query()))
    }
}
