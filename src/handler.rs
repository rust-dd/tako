use bytes::Bytes;
use http_body_util::Empty;
use hyper::{
    Request, Response,
    body::{Body, Incoming},
};

#[async_trait::async_trait]
pub trait Handler<B>: Send + Sync + 'static
where
    B: Body + From<Empty<Bytes>> + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    async fn call(&self, req: Request<Incoming>) -> Response<B>;
}

#[async_trait::async_trait]
impl<B, F, Fut> Handler<B> for F
where
    B: Body + From<Empty<Bytes>> + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
    F: Fn(Request<Incoming>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = Response<B>> + Send + 'static,
{
    async fn call(&self, req: Request<Incoming>) -> Response<B> {
        (self)(req).await
    }
}
