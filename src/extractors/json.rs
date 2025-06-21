/// This module provides the `Json` extractor, which is used to deserialize the body of a request into a strongly-typed JSON object.
use std::pin::Pin;

use anyhow::Result;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;

use crate::{extractors::FromRequest, types::Request};

/// The `Json` struct is an extractor that wraps a deserialized JSON object of type `T`.
///
/// # Example
///
/// ```rust
/// use tako::extractors::json::Json;
/// use tako::types::Request;
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct MyData {
///     field: String,
/// }
///
/// async fn handle_request(mut req: Request) -> anyhow::Result<()> {
///     let json_data: Json<MyData> = Json::from_request(&mut req).await?;
///     // Use the extracted JSON data here
///     Ok(())
/// }
/// ```
pub struct Json<T>(pub T);

/// Implementation of the `FromRequest` trait for the `Json` extractor.
///
/// This allows the `Json` extractor to be used in request handlers to deserialize
/// the body of the request into a strongly-typed JSON object.
impl<'a, T> FromRequest<'a> for Json<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    /// Extracts and deserializes the body of the request into a JSON object of type `T`.
    ///
    /// # Arguments
    ///
    /// * `req` - A mutable reference to the incoming request.
    ///
    /// # Returns
    ///
    /// A future that resolves to a `Result` containing the `Json` extractor.
    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move {
            let bytes = req.body_mut().collect().await?.to_bytes();
            let data = serde_json::from_slice(&bytes)?;
            Ok(Json(data))
        })
    }
}
