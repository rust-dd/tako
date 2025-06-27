/// This module provides the `Params` extractor, which is used to extract and deserialize
/// path parameters from a request.
///
/// The `Params` extractor is particularly useful for handling dynamic segments in request
/// paths, allowing them to be easily converted into strongly-typed structures.
use std::collections::HashMap;

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{extractors::FromRequest, types::Request};

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

/// Implementation of the `FromRequest` trait for the `Params` extractor.
///
/// This allows the `Params` extractor to be used in request handlers to deserialize
/// path parameters into strongly-typed structures.
impl<'a, T> FromRequest<'a> for Params<T>
where
    T: DeserializeOwned + Send + 'a,
{
    /// Extracts and deserializes path parameters from the request.
    ///
    /// # Arguments
    ///
    /// * `req` - A mutable reference to the incoming request.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `Result` containing the `Params` extractor.
    fn from_request(req: &'a Request) -> Result<Self> {
        let map = req
            .extensions()
            .get::<PathParams>()
            .expect("PathParams not found");

        let coerced = Self::coerce_params(&map.0);
        let value = Value::Object(coerced);
        let parsed = serde_json::from_value::<T>(value)?;

        Ok(Params(parsed))
    }
}

impl<T> Params<T> {
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
            } else {
                Value::String(v.clone())
            };

            result.insert(k.clone(), val);
        }

        result
    }
}
