use std::sync::Arc;

use crate::{
    handler::BoxHandler,
    types::{BoxMiddleware, Request, Response},
};

pub struct Next {
    pub middlewares: Arc<Vec<BoxMiddleware>>,
    pub endpoint: Arc<BoxHandler>,
}

impl Next {
    pub async fn run(self, req: Request) -> Response {
        if let Some((mw, rest)) = self.middlewares.split_first() {
            let rest = Arc::new(rest.to_vec());
            mw(
                req,
                Next {
                    middlewares: rest,
                    endpoint: self.endpoint.clone(),
                },
            )
            .await
        } else {
            self.endpoint.call(req).await
        }
    }
}
