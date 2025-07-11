use std::{pin::Pin, sync::Arc};

use crate::{
    handler::BoxHandler,
    types::{BoxMiddleware, Request, Response},
};

pub mod basic_auth;
pub mod bearer_auth;
pub mod body_limit;
pub mod jwt_auth;

pub trait IntoMiddleware {
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static;
}

/// The `Next` struct represents the next middleware or endpoint in the chain.
/// It is used to manage the flow of execution through the middleware stack.
///
/// # Fields
/// - `middlewares`: A shared reference to the remaining middlewares in the chain.
/// - `endpoint`: A shared reference to the final endpoint to be called if no middlewares remain.
pub struct Next {
    pub middlewares: Arc<Vec<BoxMiddleware>>,
    pub endpoint: Arc<BoxHandler>,
}

impl Next {
    /// Executes the next middleware or endpoint in the chain.
    ///
    /// # Parameters
    /// - `req`: The incoming HTTP request to be processed.
    ///
    /// # Returns
    /// A `Response` generated by either a middleware or the final endpoint.
    ///
    /// # Behavior
    /// - If there are remaining middlewares, the first middleware is executed, and the rest are passed along.
    /// - If no middlewares remain, the final endpoint is called to handle the request.
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
