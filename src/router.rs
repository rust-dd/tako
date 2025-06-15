use ahash::AHashMap;
use bytes::Bytes;
use http_body_util::Empty;
use hyper::{
    Method, Request, Response,
    body::{Body, Incoming},
};

use crate::handler::Handler;

pub struct Router<B>
where
    B: Body + From<Empty<Bytes>> + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    routes: AHashMap<(Method, String), Box<dyn Handler<B>>>,
}

impl<B> Router<B>
where
    B: Body + From<Empty<Bytes>> + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    pub fn new() -> Self {
        Self {
            routes: AHashMap::default(),
        }
    }

    pub fn route<H>(&mut self, method: Method, path: &str, handler: H)
    where
        H: Handler<B>,
    {
        self.routes
            .insert((method, path.to_owned()), Box::new(handler));
    }

    pub async fn dispatch(&self, req: Request<Incoming>) -> Response<B> {
        let key = (req.method().clone(), req.uri().path().to_owned());

        if let Some(h) = self.routes.get(&key) {
            h.call(req).await
        } else {
            Response::builder()
                .status(404)
                .body(Empty::<Bytes>::new().into())
                .unwrap()
        }
    }
}
