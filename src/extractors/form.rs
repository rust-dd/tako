use std::collections::HashMap;

use http_body_util::BodyExt;
use serde::de::DeserializeOwned;

use crate::{extractors::AsyncFromRequestMut, types::Request};

/// Represents a form extracted from an HTTP request body.
///
/// This generic struct wraps the deserialized form data of type `T`.
pub struct Form<T>(pub T);

/// Implementation of the `AsyncFromRequestMut` trait for extracting form data from an HTTP request.
///
/// This implementation supports asynchronous extraction of form data from requests with the
/// `application/x-www-form-urlencoded` content type. The extracted data is deserialized into
/// the generic type `T`.
impl<'a, T> AsyncFromRequestMut<'a> for Form<T>
where
    T: DeserializeOwned + Send,
{
    /// Extracts form data from the HTTP request body.
    ///
    /// # Arguments
    /// * `req` - A mutable reference to the HTTP request from which the form data is extracted.
    ///
    /// # Returns
    /// * `Ok(Form<T>)` - If the request contains valid form data that can be deserialized into type `T`.
    /// * `Err` - If the content type is invalid or the form data cannot be parsed or deserialized.
    async fn from_request(req: &'a mut Request) -> anyhow::Result<Self> {
        if req
            .headers()
            .get(hyper::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            != Some("application/x-www-form-urlencoded")
        {
            return Err(anyhow::anyhow!("Invalid content type"));
        }

        let body_bytes = req.body_mut().collect().await?.to_bytes();
        let body_str = std::str::from_utf8(&body_bytes)?;
        let form = url::form_urlencoded::parse(body_str.as_bytes())
            .into_owned()
            .collect::<Vec<(String, String)>>();
        let form_map = HashMap::<String, String>::from_iter(form);
        let json = serde_json::to_value(form_map)?;
        let form = serde_json::from_value::<T>(json)?;
        Ok(Form(form))
    }
}
