use http::Method;

use crate::{
    handler::BoxedHandler,
    middleware::Middleware,
    types::{AppState, BoxedRequestFuture},
};

pub struct Route<'a, S>
where
    S: AppState,
{
    pub path: &'a str,
    pub method: Method,
    pub handler: BoxedHandler<S>,
    pub middlewares: Vec<Box<dyn Middleware<S, Future = BoxedRequestFuture>>>,
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

    pub fn middleware<M>(mut self, middleware: M) -> Self
    where
        M: Middleware<S, Future = BoxedRequestFuture>,
    {
        self.middlewares.push(Box::new(middleware));
        self
    }
}
