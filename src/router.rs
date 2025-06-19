use std::sync::Arc;

use dashmap::DashMap;
use hyper::Method;

use crate::{
    body::TakoBody,
    extractors::params::PathParams,
    handler::{BoxedHandler, Handler},
    responder::Responder,
    route::Route,
    state::set_state,
    types::{BoxedMiddleware, BoxedRequestFuture, Request, Response},
};

pub struct Router {
    routes: DashMap<(Method, String), Arc<Route>>,
    middlewares: Vec<BoxedMiddleware>,
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
            None,
        ));
        self.routes
            .insert((method.clone(), path.to_owned()), route.clone());
        route
    }

    pub fn route_with_tsr<H>(&mut self, method: Method, path: &str, handler: H) -> Arc<Route>
    where
        H: Handler + Clone + 'static,
    {
        if path == "/" {
            panic!("Cannot route with TSR for root path");
        }

        let route = Arc::new(Route::new(
            path.to_string(),
            method.clone(),
            BoxedHandler::new(handler),
            Some(true),
        ));
        self.routes
            .insert((method.clone(), path.to_owned()), route.clone());
        route
    }

    pub async fn dispatch(&self, mut req: Request) -> Response {
        let method = req.method();
        let path = req.uri().path();

        for route in self.routes.iter() {
            if &route.method != method {
                continue;
            }

            if let Some(params) = route.match_path(path) {
                req.extensions_mut().insert(PathParams(params));

                let r_mws = route.middlewares.read().await;
                let mws = self.middlewares.iter().chain(r_mws.iter()).rev();

                for mw in mws {
                    match mw(req).await {
                        Ok(r) => req = r,
                        Err(resp) => return resp,
                    }
                }

                return route.handler.call(req).await;
            }
        }

        let tsr_path = if path.ends_with('/') {
            path.trim_end_matches('/').to_string()
        } else {
            format!("{}/", path)
        };

        for route in self.routes.iter() {
            if &route.method == method && route.tsr && route.match_path(&tsr_path).is_some() {
                return hyper::Response::builder()
                    .status(307)
                    .header("Location", tsr_path)
                    .body(TakoBody::empty())
                    .unwrap();
            }
        }

        hyper::Response::builder()
            .status(404)
            .body(TakoBody::empty())
            .unwrap()
    }

    pub fn state<T: Clone + Send + Sync + 'static>(&mut self, key: &str, value: T) {
        set_state(key, value);
    }

    pub fn middleware<F, Fut, R>(&mut self, f: F)
    where
        F: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
        Fut: Future<Output = Result<Request, R>> + Send + 'static,
        R: Responder + Send + 'static,
    {
        let mw: BoxedMiddleware = Box::new(move |req: Request| {
            let f = f.clone();
            Box::pin(async move {
                match f(req).await {
                    Ok(r) => Ok(r),
                    Err(e) => Err(e.into_response()),
                }
            })
        });

        self.middlewares.push(mw);
    }
}
