use ahash::AHashMap;
use hyper::Method;

use crate::{
    body::TakoBody,
    handler::{BoxedHandler, Handler},
    route::Route,
    types::{AppState, BoxedRequestFuture, Request, Response},
};

pub struct Router<'a, S>
where
    S: AppState + Clone + Default,
{
    routes: AHashMap<(Method, String), Route<'a, S>>,
    state: S,
    middlewares: Vec<Box<dyn Fn(Request) -> BoxedRequestFuture + Send + Sync + 'static>>,
}

impl<'a, S> Router<'a, S>
where
    S: AppState + Clone + Default,
{
    pub fn new() -> Self {
        Self {
            routes: AHashMap::default(),
            state: S::default(),
            middlewares: Vec::new(),
        }
    }

    pub fn route<H, T>(&mut self, method: Method, path: &'a str, handler: H) -> Route<'a, S>
    where
        H: Handler<T, S> + Clone + 'static,
    {
        let route = self.routes.insert(
            (method.clone(), path.to_owned()),
            Route::new(path, method, BoxedHandler::new(handler)),
        );
        let route = route.unwrap();
        route
    }

    pub async fn dispatch(&self, req: Request) -> Response {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(h) = self.routes.get(&key) {
            h.handler.call(req, self.state.clone()).await
        } else {
            hyper::Response::builder()
                .status(404)
                .body(TakoBody::empty())
                .unwrap()
        }
    }

    pub fn state(&mut self, state: S) {
        self.state = state;
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
