use http::{Request, Response};
use hyper::body::Incoming;

use crate::{
    body::TakoBody,
    types::{AppState, Fut},
};

#[async_trait::async_trait]
pub trait Middleware<S>: Send + Sync {
    async fn handle(&self, req: Request<Incoming>, state: &S) -> Response<TakoBody>;
}

pub struct Next<'a, S> {
    pub idx: usize,
    pub chain: &'a [Box<dyn Middleware<S>>],
    pub final_step: Box<dyn FnOnce(Request<Incoming>, S) -> Fut<'a> + 'a>,
}

impl<'a, S> Next<'a, S>
where
    S: AppState,
{
    pub async fn run(mut self, req: Request<Incoming>, state: S) -> Response<TakoBody> {
        if self.idx < self.chain.len() {
            let mw = &self.chain[self.idx];
            self.idx += 1;
            mw.handle(req, &state).await
        } else {
            (self.final_step)(req, state).await
        }
    }
}
