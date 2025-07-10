use std::collections::HashMap;

use http_body_util::BodyExt;
use serde::de::DeserializeOwned;

use crate::{extractors::AsyncFromRequestMut, types::Request};

pub struct Form<T>(pub T);

impl<'a, T> AsyncFromRequestMut<'a> for Form<T>
where
    T: DeserializeOwned + Send,
{
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
