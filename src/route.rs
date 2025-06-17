use std::pin::Pin;

use http::Method;

use crate::{
    handler::BoxedHandler,
    middleware::Middleware,
    types::{AppState, BoxedRequestFuture, Request},
};

pub struct Route<'a, S>
where
    S: AppState,
{
    pub path: &'a str,
    pub method: Method,
    pub handler: BoxedHandler<S>,
    middlewares: Vec<Box<dyn Fn(Request) -> BoxedRequestFuture>>,
}

impl<'a, S> Route<'a, S>
where
    S: AppState,
{
    pub fn new(path: &'a str, method: Method, handler: BoxedHandler<S>) -> Self {
        Self {
            path,
            method,
            handler,
            middlewares: Vec::new(),
        }
    }

    pub fn middleware<F, Fut>(mut self, f: F) -> Self
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Request> + Send + 'static,
    {
        let wrapped = move |req: Request| -> Pin<Box<dyn Future<Output = Request> + Send>> {
            Box::pin(f(req))
        };

        self.middlewares.push(Box::new(wrapped));
        self
    }
}
