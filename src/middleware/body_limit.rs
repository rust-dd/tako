use std::{pin::Pin, sync::Arc};

use http::StatusCode;

use crate::{
    middleware::Next,
    responder::Responder,
    types::{Request, Response},
};

pub struct Config<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    limit: Option<usize>,
    dynamic_limit: Option<F>,
}

impl<F> Config<F>
where
    F: Fn(&Request) -> usize + Send + Sync + 'static,
{
    pub fn new(limit: usize, dynamic_limit: Option<F>) -> Self {
        Self {
            limit: Some(limit),
            dynamic_limit,
        }
    }

    pub fn with_dynamic_limit(f: F) -> Self {
        Self {
            limit: None,
            dynamic_limit: Some(f),
        }
    }

    pub fn new_with_dynamic(limit: usize, f: F) -> Self {
        Self {
            limit: Some(limit),
            dynamic_limit: Some(f),
        }
    }

    pub fn into_middleware(
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
                // Végső határérték (dinamikus → statikus → fallback 10 MiB).
                let limit = dynamic_limit
                    .as_ref()
                    .map(|f| f(&req))
                    .or(static_limit)
                    .unwrap_or(10 * 1024 * 1024);

                // 1) Gyors kilépés a Content-Length alapján.
                if let Some(len) = req
                    .headers()
                    .get(hyper::header::CONTENT_LENGTH)
                    .and_then(|v| v.to_str().ok())
                    .and_then(|s| s.parse::<usize>().ok())
                {
                    if len > limit {
                        return (
                            StatusCode::PAYLOAD_TOO_LARGE,
                            "Body exceeds allowed size".to_string(),
                        )
                            .into_response();
                    }
                }

                // TODO: maybe add runtime limit

                next.run(req).await.into_response()
            })
        }
    }
}
