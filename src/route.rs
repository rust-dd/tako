use http::Method;

use crate::{handler::BoxedHandler, types::BoxedRequestFuture};

pub struct Route<'a, S> {
    pub path: &'a str,
    pub method: Method,
    pub handler: BoxedHandler<S>,
    pub middlewares: Vec<Box<dyn Fn() -> BoxedRequestFuture>>,
}

impl<'a, S> Route<'a, S> {
    pub fn new(path: &'a str, method: Method, handler: BoxedHandler<S>) -> Self {
        Self {
            path,
            method,
            handler,
            middlewares: Vec::new(),
        }
    }

    pub fn middleware(mut self, middleware: Box<dyn Fn() -> BoxedRequestFuture>) -> Self {
        self.middlewares.push(middleware);
        self
    }
}
