use std::{pin::Pin, sync::Arc};

use crate::{
    handler::BoxedHandler,
    types::{Request, Response},
};

type BoxFutureResp<'a> = Pin<Box<dyn Future<Output = Response> + Send + 'a>>;
pub type BoxedMiddleware =
    Arc<dyn for<'a> Fn(Request, Next<'a>) -> BoxFutureResp<'a> + Send + Sync>;

pub struct Next<'a> {
    pub middlewares: &'a [BoxedMiddleware],
    pub endpoint: &'a BoxedHandler,
}

impl<'a> Next<'a> {
    pub async fn run(self, req: Request) -> Response {
        if let Some((mw, rest)) = self.middlewares.split_first() {
            mw(
                req,
                Next {
                    middlewares: rest,
                    endpoint: self.endpoint,
                },
            )
            .await
        } else {
            self.endpoint.call(req).await
        }
    }
}
