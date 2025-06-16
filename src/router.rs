use ahash::AHashMap;
use hyper::{Method, Request, Response, body::Incoming};

use crate::{
    body::TakoBody,
    handler::Handler,
    middleware::{Middleware, Next},
    route::{Route, RouteCondig},
    types::{AppState, Fut},
};

pub struct Router<S>
where
    S: AppState,
{
    pub(crate) routes: AHashMap<(Method, String), Route<S>>,
    middlewares: Vec<Box<dyn Middleware<S>>>,
    state: S,
}

impl<S> Router<S>
where
    S: AppState,
{
    pub fn new() -> Self {
        Self {
            routes: AHashMap::default(),
            middlewares: Vec::new(),
            state: S::default(),
        }
    }

    pub fn route<H>(&mut self, method: Method, path: &str, handler: H) -> RouteCondig<'_, S>
    where
        H: Handler<S> + 'static,
    {
        self.routes.insert(
            (method.clone(), path.to_owned()),
            Route {
                handler: Box::new(handler),
                middlewares: Vec::new(),
            },
        );
        RouteCondig {
            router: self,
            key: (method, path.to_owned()),
        }
    }

    pub async fn dispatch(&self, req: Request<Incoming>) -> Response<TakoBody> {
        let key = (req.method().clone(), req.uri().path().to_owned());

        let route = match self.routes.get(&key) {
            Some(r) => r,
            None => {
                return Response::builder()
                    .status(404)
                    .body(TakoBody::empty())
                    .unwrap();
            }
        };

        let chain = self
            .middlewares
            .into_iter()
            .chain(route.middlewares.into_iter())
            .collect::<Vec<_>>();

        let final_step = {
            Box::new(move |req, state: _| {
                Box::pin(async move { route.handler.call(req, state.into()).await as Fut<'_> })
            })
        };

        Next {
            idx: 0,
            chain: &chain,
            final_step,
        }
        .run(req, self.state.clone())
        .await
    }

    pub fn state(&mut self, state: S) {
        self.state = state
    }

    pub fn middleware<M>(&mut self, middleware: M)
    where
        M: Middleware<S> + 'static,
    {
        self.middlewares.push(Box::new(middleware));
    }
}
