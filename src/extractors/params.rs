use std::{collections::HashMap, pin::Pin};

use anyhow::Result;
use serde::de::DeserializeOwned;
use serde_json::{Map, Value};

use crate::{extractors::FromRequest, types::Request};

#[derive(Clone, Default)]
pub(crate) struct PathParams(pub HashMap<String, String>);

pub struct Params<T>(pub T);

impl<'a, T> FromRequest<'a> for Params<T>
where
    T: DeserializeOwned + Send + 'a,
{
    type Fut = Pin<Box<dyn Future<Output = Result<Self>> + Send + 'a>>;

    fn from_request(req: &'a mut Request) -> Self::Fut {
        Box::pin(async move {
            let map = req
                .extensions()
                .get::<PathParams>()
                .expect("PathParams not found");

            let coerced = Self::coerce_params(&map.0);
            let value = Value::Object(coerced);
            let parsed = serde_json::from_value::<T>(value)?;

            Ok(Params(parsed))
        })
    }
}

impl<T> Params<T> {
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
