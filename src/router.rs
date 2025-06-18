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

pub struct Router {
    routes: DashMap<(Method, String), Arc<Route>>,
    middlewares: Vec<Box<dyn Fn(Request) -> BoxedRequestFuture + Send + Sync + 'static>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: DashMap::default(),
            middlewares: Vec::new(),
        }
    }

    pub fn route<H>(&mut self, method: Method, path: &str, handler: H) -> Arc<Route>
    where
        H: Handler + Clone + 'static,
    {
        let route = Arc::new(Route::new(
            path.to_string(),
            method.clone(),
            BoxedHandler::new(handler),
        ));
        self.routes
            .insert((method.clone(), path.to_owned()), route.clone());
        route
    }

    pub async fn dispatch(&self, mut req: Request) -> Response {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(route) = self.routes.get(&key).map(|r| r.clone()) {
            let r_mws = route.middlewares.read().await;
            let mws = self.middlewares.iter().chain(r_mws.iter()).rev();

            for mw in mws {
                req = mw(req).await;
            }

            route.handler.call(req).await
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
