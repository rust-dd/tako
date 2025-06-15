use ahash::AHashMap;
use hyper::{Method, Request, Response, body::Incoming};

use crate::{body::TakoBody, handler::Handler};

pub struct Router {
    routes: AHashMap<(Method, String), Box<dyn Handler>>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            routes: AHashMap::default(),
        }
    }

    pub fn route<H>(&mut self, method: Method, path: &str, handler: H)
    where
        H: Handler,
    {
        self.routes
            .insert((method, path.to_owned()), Box::new(handler));
    }

    pub async fn dispatch(&self, req: Request<Incoming>) -> Response<TakoBody> {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(h) = self.routes.get(&key) {
            h.call(req).await
        } else {
            Response::builder()
                .status(404)
                .body(TakoBody::empty())
                .unwrap()
        }
    }
}
