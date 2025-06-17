use crate::types::{AppState, Request};

pub trait Middleware<S>: Send + Sync + 'static
where
    S: AppState,
{
    type Future: Future<Output = Request> + Send + 'static;

    fn call(&self, request: Request) -> Self::Future;
}

pub struct Next;
