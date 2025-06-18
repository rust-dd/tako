use std::sync::Arc;

use http::Method;
use tokio::sync::RwLock;

use crate::{
    handler::BoxedHandler,
    types::{BoxedRequestFuture, Request},
};

pub struct Route {
    pub path: String,
    pub method: Method,
    pub handler: BoxedHandler,
    pub middlewares:
        RwLock<Vec<Box<dyn Fn(Request) -> BoxedRequestFuture + Send + Sync + 'static>>>,
}

impl Route {
    pub fn new(path: String, method: Method, handler: BoxedHandler) -> Self {
        Self {
            path,
            method,
            handler,
            middlewares: RwLock::new(Vec::new()),
        }
    }

    pub fn middleware<F, Fut>(self: Arc<Self>, f: F) -> Arc<Self>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Request> + Send + 'static,
    {
        let this = self.clone();

        tokio::spawn(async move {
            let mut lock = this.middlewares.write().await;
            lock.push(Box::new(move |req: Request| -> BoxedRequestFuture {
                Box::pin(f(req))
            }));
        });

        self
    }
}
