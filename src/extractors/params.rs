/// This module provides the `Params` extractor, which is used to extract and deserialize
/// path parameters from a request.
///
/// The `Params` extractor is particularly useful for handling dynamic segments in request
/// paths, allowing them to be easily converted into strongly-typed structures.
use std::{collections::HashMap, future::ready};

use http::StatusCode;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// A helper struct that stores path parameters as a `HashMap`.
///
/// This struct is used internally to manage path parameters extracted from the request.
#[derive(Clone, Default)]
pub(crate) struct PathParams(pub HashMap<String, String>);

/// The `Params` struct is an extractor that wraps a deserialized representation of path parameters.
///
/// # Example
///
/// ```rust
/// use tako::extractors::params::Params;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct MyParams {
///     id: u64,
///     name: String,
/// }
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let params = Params::<MyParams>::from_request(&mut req).await?;
///     // Use the extracted and deserialized parameters here
///     Ok(())
/// }
/// ```
pub struct Params<T>(pub T);

/// Error type for path parameter extraction.
#[derive(Debug)]
pub enum ParamsError {
    MissingPathParams,
    DeserializationError(String),
}

impl Responder for ParamsError {
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

    /// Converts a `HashMap` of string parameters into a `Map` of JSON values.
    ///
    /// This function attempts to coerce string values into numeric types where possible.
    ///
    /// # Arguments
    ///
    /// * `map` - A reference to the `HashMap` containing string parameters.
    ///
    /// # Returns
    ///
    /// A `Map` where the values are JSON-compatible types.
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
