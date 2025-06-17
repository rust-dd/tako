use ahash::AHashMap;
use hyper::Method;

use crate::{
    body::TakoBody,
    handler::{BoxedHandler, Handler},
    types::{AppState, BoxedHandlerFuture, Request, Response},
};

pub struct Router<S>
where
    S: AppState + Clone + Default,
{
    routes: AHashMap<(Method, String), BoxedHandler<S>>,
    state: S,
    middlewares: Vec<Box<dyn FnOnce() -> () + Send + 'static>>,
}

impl<S> Router<S>
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

    pub fn route<H, T>(&mut self, method: Method, path: &str, handler: H)
    where
        H: Handler<T, S> + Clone + 'static,
    {
        self.routes
            .insert((method, path.to_owned()), BoxedHandler::new(handler));
    }

    pub async fn dispatch(&self, req: Request) -> Response {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(h) = self.routes.get(&key) {
            h.call(req, self.state.clone()).await
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
}
