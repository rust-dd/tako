use std::{future::Future, pin::Pin, sync::Arc};

use http::{StatusCode, header::CONTENT_LENGTH};

use crate::{
    middleware::{IntoMiddleware, Next},
    responder::Responder,
    types::{Request, Response},
};

pub struct BodyLimit<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    limit: Option<usize>,
    dynamic_limit: Option<F>,
}

impl<F> BodyLimit<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    /// Create with a fixed `limit` (bytes).
    pub fn new(limit: usize) -> Self {
        Self {
            limit: Some(limit),
            dynamic_limit: None,
        }
    }

    /// Create with a dynamic limit closure.
    pub fn with_dynamic_limit(f: F) -> Self {
        Self {
            limit: None,
            dynamic_limit: Some(f),
        }
    }

    /// Create with both static and dynamic limits.
    /// The closure's value overrides the static one.
    pub fn new_with_dynamic(limit: usize, f: F) -> Self {
        Self {
            limit: Some(limit),
            dynamic_limit: Some(f),
        }
    }
}

impl<F> IntoMiddleware for BodyLimit<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static {
        let static_limit = self.limit;
        let dynamic_limit = self.dynamic_limit.map(Arc::new);

        move |req: Request, next: Next| {
            let dynamic_limit = dynamic_limit.clone();

            Box::pin(async move {
                // effective limit: dynamic → static → default 10 MiB
                let limit = dynamic_limit
                    .as_ref()
                    .map(|f| f(&req))
                    .or(static_limit)
                    .unwrap_or(10 * 1024 * 1024);

                // Fast‑path rejection via Content‑Length.
                if let Some(len) = req
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if len > limit {
                        return (StatusCode::PAYLOAD_TOO_LARGE, "Body exceeds allowed size")
                            .into_response();
                    }
                }

                // TODO: add run‑time stream truncation if your Body supports it.

                next.run(req).await.into_response()
            })
        }
    }
}
