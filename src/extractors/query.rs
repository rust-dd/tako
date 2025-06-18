use std::{collections::HashMap, pin::Pin};

use anyhow::Result;
use serde::de::DeserializeOwned;
use url::form_urlencoded;

use crate::{extractors::FromRequest, types::Request};

pub struct Query<T>(pub T);

impl<'a, T> FromRequest<'a> for Query<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(req: &'a mut Request) -> Self::Fut {
        let query = req.uri().query().unwrap_or_default();
        let kv = form_urlencoded::parse(query.as_bytes())
            .into_owned()
            .collect::<HashMap<String, String>>();
        let value = serde_json::to_value(kv).unwrap();
        let value = serde_json::from_value::<T>(value).unwrap();

        Box::pin(async move { Ok(Query(value)) })
    }
}
