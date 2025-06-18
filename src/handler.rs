use std::{pin::Pin, sync::Arc};

use crate::{
    responder::Responder,
    types::{BoxedResponseFuture, Request, Response},
};

pub trait Handler: Send + Sync + 'static {
    type Future: Future<Output = Response> + Send + 'static;

    fn call(self, req: Request) -> Self::Future;
}

impl<F, Fut, R> Handler for F
where
    F: FnOnce(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = R> + Send,
    R: Responder,
{
    type Future = Pin<Box<dyn Future<Output = Response> + Send>>;

    fn call(self, req: Request) -> Self::Future {
        Box::pin(async move { (self)(req).await.into_response() })
    }
}

#[derive(Clone)]
pub struct BoxedHandler {
    inner: Arc<dyn Fn(Request) -> BoxedResponseFuture + Send + Sync>,
}

impl BoxedHandler {
    pub(crate) fn new<H>(h: H) -> Self
    where
        H: Handler + Clone,
    {
        let inner = Arc::new(move |req: Request| {
            let handler = h.clone();
            Box::pin(async move { handler.call(req.into()).await }) as BoxedResponseFuture
        });

        Self { inner }
    }

    pub(crate) fn call(&self, req: Request) -> BoxedResponseFuture {
        (self.inner)(req)
    }
}
