use http::Method;

use crate::handler::BoxedHandler;

pub(crate) struct Route<S> {
    pub path: String,
    pub method: Method,
    pub handler: BoxedHandler<S>,
    pub middlewares: Vec<Box<dyn FnOnce() + Send + 'static>>,
}
