use http::Method;

use crate::{handler::Handler, middleware::Middleware, router::Router, types::AppState};

pub struct Route<S> {
    pub handler: Box<dyn Handler<S>>,
    pub middlewares: Vec<Box<dyn Middleware<S>>>,
}

pub struct RouteCondig<'a, S>
where
    S: AppState,
{
    pub router: &'a mut Router<S>,
    pub key: (Method, String),
}

impl<'a, S> RouteCondig<'a, S>
where
    S: AppState,
{
    pub fn middleware<M>(self, middleware: M) -> Self
    where
        M: Middleware<S> + 'static,
    {
        if let Some(route) = self.router.routes.get_mut(&self.key) {
            route.middlewares.push(Box::new(middleware));
        }
        self
    }
}
