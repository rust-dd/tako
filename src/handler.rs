use http::Response;
use hyper::{Request, body::Incoming};

use crate::{body::TakoBody, responder::Responder};

#[async_trait::async_trait]
pub trait Handler: Send + Sync + 'static {
    async fn call(&self, req: Request<Incoming>) -> Response<TakoBody>;
}

#[async_trait::async_trait]
impl<F, Fut, R> Handler for F
where
    F: Fn(Request<Incoming>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
{
    async fn call(&self, req: Request<Incoming>) -> Response<TakoBody> {
        (self)(req).await.into_response()
    }
}
