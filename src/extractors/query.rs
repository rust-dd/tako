/// This module provides the `Query` extractor, which is used to extract query parameters from a request.
///
/// The `Query` extractor allows deserialization of query parameters into a strongly-typed structure,
/// making it easier to work with query strings in a type-safe manner.
use std::collections::HashMap;

use anyhow::Result;
use serde::de::DeserializeOwned;
use url::form_urlencoded;

use crate::{extractors::FromRequest, types::Request};

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

/// Implementation of the `FromRequest` trait for the `Query` extractor.
///
/// This allows the `Query` extractor to be used in request handlers to easily access
/// and deserialize query parameters from the request URI.
impl<'a, T> FromRequest<'a> for Query<T>
where
    T: DeserializeOwned + Send + 'a,
{
    /// Extracts and deserializes query parameters from the request URI.
    ///
    /// # Arguments
    ///
    /// * `req` - A mutable reference to the incoming request.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `Result` containing the `Query` extractor.
    fn from_request(req: &'a Request) -> Result<Self> {
        let query = req.uri().query().unwrap_or_default();
        let kv = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>();
        let value = serde_json::to_value(kv).unwrap();
        let value = serde_json::from_value::<T>(value).unwrap();

        Ok(Query(value))
    }
}
