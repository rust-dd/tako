use std::sync::Arc;

use dashmap::DashMap;
use hyper::Method;

use crate::{
    body::TakoBody,
    handler::{BoxedHandler, Handler},
    route::Route,
    state::set_state,
    types::{BoxedRequestFuture, Request, Response},
};

pub struct Router<'a> {
    routes: DashMap<(Method, String), Arc<Route<'a>>>,
    middlewares: Vec<Box<dyn Fn(Request) -> BoxedRequestFuture + Send + Sync + 'static>>,
}

impl<'a> Router<'a> {
    pub fn new() -> Self {
        Self {
            routes: DashMap::default(),
            middlewares: Vec::new(),
        }
    }

    pub fn route<H>(&mut self, method: Method, path: &'a str, handler: H) -> Arc<Route<'a>>
    where
        H: Handler + Clone + 'static,
    {
        let route = Arc::new(Route::new(path, method.clone(), BoxedHandler::new(handler)));
        self.routes
            .insert((method.clone(), path.to_owned()), route.clone());
        route
    }

    pub async fn dispatch(&self, req: Request) -> Response {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(h) = self.routes.get(&key) {
            h.handler.call(req).await
        } else {
            hyper::Response::builder()
                .status(404)
                .body(TakoBody::empty())
                .unwrap()
        }
    }

    pub fn state<T: Clone + Send + Sync + 'static>(&mut self, key: &str, value: T) {
        set_state(key, value);
    }

    pub fn middleware<F, Fut>(&mut self, f: F)
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Request> + Send + Sync + 'static,
    {
        self.middlewares
            .push(Box::new(move |req: Request| -> BoxedRequestFuture {
                Box::pin(f(req))
            }));
    }
}
