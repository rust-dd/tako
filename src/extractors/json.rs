use std::pin::Pin;

use anyhow::Result;
use http_body_util::BodyExt;
use serde::de::DeserializeOwned;

use crate::{extractors::FromRequest, types::Request};

pub struct Json<T>(pub T);

impl<'a, T> FromRequest<'a> for Json<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move {
            let bytes = req.body_mut().collect().await?.to_bytes();
            let data = serde_json::from_slice(&bytes)?;
            Ok(Json(data))
        })
    }
}
