use ahash::AHashMap;
use hyper::{Method, Request, Response, body::Incoming};

use crate::{body::TakoBody, handler::Handler, types::AppState};

pub struct Router<S>
where
    S: AppState,
{
    routes: AHashMap<(Method, String), Box<dyn Handler<S>>>,
    state: S,
}

impl<S> Router<S>
where
    S: AppState,
{
    pub fn new() -> Self {
        Self {
            routes: AHashMap::default(),
            state: S::default(),
        }
    }

    pub fn route<H>(&mut self, method: Method, path: &str, handler: H)
    where
        H: Handler<S>,
    {
        self.routes
            .insert((method, path.to_owned()), Box::new(handler));
    }

    pub async fn dispatch(&self, req: Request<Incoming>) -> Response<TakoBody> {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(h) = self.routes.get(&key) {
            h.call(req, self.state.clone().into()).await
        } else {
            Response::builder()
                .status(404)
                .body(TakoBody::empty())
                .unwrap()
        }
    }

    pub fn state(&mut self, state: S) {
        self.state = state
    }
}
