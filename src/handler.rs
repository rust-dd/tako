use http::Response;
use hyper::{Request, body::Incoming};

use crate::{body::TakoBody, extractors::state::State, responder::Responder};

#[async_trait::async_trait]
pub trait Handler<S>: Send + Sync + 'static {
    async fn call(&self, req: Request<Incoming>, state: State<S>) -> Response<TakoBody>;
}

#[async_trait::async_trait]
impl<F, Fut, R, S> Handler<S> for F
where
    F: Fn(Request<Incoming>, State<S>) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
    S: Send + Sync + 'static,
{
    async fn call(&self, req: Request<Incoming>, state: State<S>) -> Response<TakoBody> {
        (self)(req, state).await.into_response()
    }
}
