use crate::types::{AppState, Request};

#[async_trait::async_trait]
pub trait Middleware<S>: Send + Sync + 'static
where
    S: AppState,
{
    async fn call(&self, _req: Request) -> Request;
}

pub struct Next;
