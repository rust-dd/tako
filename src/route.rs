use std::sync::{Arc, RwLock};

use http::Method;

use crate::{
    handler::BoxedHandler,
    types::{BoxedRequestFuture, Request},
};

pub struct Route<'a> {
    pub path: &'a str,
    pub method: Method,
    pub handler: BoxedHandler,
    middlewares: RwLock<Vec<Box<dyn Fn(Request) -> BoxedRequestFuture + Send + Sync + 'static>>>,
}

impl<'a> Route<'a> {
    pub fn new(path: &'a str, method: Method, handler: BoxedHandler) -> Self {
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
        Fut: Future<Output = Request> + Send + Sync + 'static,
    {
        let mut lock = self.middlewares.write().unwrap();
        lock.push(Box::new(move |req: Request| -> BoxedRequestFuture {
            Box::pin(f(req))
        }));
        self.clone()
    }
}
