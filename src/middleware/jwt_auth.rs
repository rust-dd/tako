use std::{collections::HashMap, future::Future, pin::Pin, sync::Arc};

use http::{StatusCode, header::AUTHORIZATION};
use serde::de::DeserializeOwned;

use crate::{
    middleware::{IntoMiddleware, Next},
    responder::Responder,
    types::{Request, Response},
};

pub struct JwtAuth<T>
where
    T: Clone + DeserializeOwned + Send + Sync,
{
    keys: Arc<HashMap<Algorithm, DecodingKey>>,
    allowed_algos: Vec<Algorithm>,
    _phantom: std::marker::PhantomData<T>,
}

impl<T> JwtAuth<T>
where
    T: Clone + DeserializeOwned + Send + Sync,
{
    pub fn new(keys: HashMap<Algorithm, DecodingKey>) -> Self {
        let allowed_algos = keys.keys().cloned().collect();
        Self {
            keys: Arc::new(keys),
            allowed_algos,
            _phantom: std::marker::PhantomData,
        }
    }
}

impl<T> IntoMiddleware for JwtAuth<T>
where
    T: Clone + DeserializeOwned + Send + Sync + 'static,
{
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let keys = self.keys.clone();
        let allowed_algos = self.allowed_algos.clone();

        move |mut req: Request, next: Next| {
            let keys = keys.clone();
            let allowed_algos = allowed_algos.clone();

            Box::pin(async move {
                let token_opt = req
                    .headers()
                    .get(AUTHORIZATION)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.strip_prefix("Bearer "))
                    .map(str::trim);

                let token = match token_opt {
                    Some(t) => t,
                    None => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            "Missing or invalid Authorization header",
                        )
                            .into_response();
                    }
                };

                let header = match decode_header(token) {
                    Ok(h) => h,
                    Err(_) => {
                        return (StatusCode::UNAUTHORIZED, "Invalid JWT header").into_response();
                    }
                };

                let alg = header.alg;
                if !allowed_algos.contains(&alg) {
                    return (
                        StatusCode::UNAUTHORIZED,
                        format!("Algorithm {:?} not allowed", alg),
                    )
                        .into_response();
                }

                let key = match keys.get(&alg) {
                    Some(k) => k,
                    None => {
                        return (
                            StatusCode::UNAUTHORIZED,
                            format!("No key for algorithm {:?}", alg),
                        )
                            .into_response();
                    }
                };

                let mut validation = Validation::new(alg);
                validation.algorithms = allowed_algos.clone();

                let decoded = decode::<T>(token, key, &validation);

                let token_data = match decoded {
                    Ok(data) => data,
                    Err(err) => {
                        return (StatusCode::UNAUTHORIZED, format!("Invalid token: {}", err))
                            .into_response();
                    }
                };

                req.extensions_mut().insert(token_data.claims);
                next.run(req).await.into_response()
            })
        }
    }
}
