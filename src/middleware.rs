//! Middleware system for request and response processing pipelines.
//!
//! This module provides the core middleware infrastructure for Tako, allowing you to
//! compose request processing pipelines. Middleware can modify requests, responses,
//! or perform side effects like logging, authentication, or rate limiting. The `Next`
//! struct manages the execution flow through the middleware chain to the final handler.
//!
//! # Examples
//!
//! ```rust
//! use tako::{middleware::Next, types::{Request, Response}};
//! use std::{pin::Pin, future::Future};
//!
//! async fn logging_middleware(req: Request, next: Next) -> Response {
//!     println!("Processing request to: {}", req.uri());
//!     let response = next.run(req).await;
//!     println!("Response status: {}", response.status());
//!     response
//! }
//! ```

use std::{future::Future, pin::Pin, sync::Arc};

use crate::{
    handler::BoxHandler,
    types::{BoxMiddleware, Request, Response},
};

pub mod basic_auth;
pub mod bearer_auth;
pub mod body_limit;
pub mod jwt_auth;

/// Trait for converting types into middleware functions.
///
/// This trait allows various types to be converted into middleware that can be used
/// in the Tako middleware pipeline. Middleware functions take a request and the next
/// middleware in the chain, returning a future that resolves to a response.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::{IntoMiddleware, Next};
/// use tako::types::{Request, Response};
/// use std::{pin::Pin, future::Future};
///
/// struct LoggingMiddleware;
///
/// impl IntoMiddleware for LoggingMiddleware {
///     fn into_middleware(
///         self,
///     ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
///     + Clone + Send + Sync + 'static {
///         |req, next| {
///             Box::pin(async move {
///                 println!("Request: {}", req.uri());
///                 next.run(req).await
///             })
///         }
///     }
/// }
/// ```
pub trait IntoMiddleware {
    fn into_middleware(
        self,
    ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
    + Clone
    + Send
    + Sync
    + 'static;
}

/// Represents the next step in the middleware execution chain.
///
/// The `Next` struct manages the flow of execution through a middleware stack,
/// ensuring each middleware is called in order before reaching the final endpoint
/// handler. It contains references to the remaining middlewares and the final
/// endpoint to be executed.
pub struct Next {
    /// Remaining middlewares to be executed in the chain.
    pub middlewares: Arc<Vec<BoxMiddleware>>,
    /// Final endpoint handler to be called after all middlewares.
    pub endpoint: Arc<BoxHandler>,
}

impl Next {
    /// Executes the next middleware or endpoint in the chain.
    ///
    /// This method processes the middleware chain by either calling the next middleware
    /// (if any remain) or the final endpoint handler. It maintains the proper execution
    /// order and passes the request through each layer of the middleware stack.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::middleware::Next;
    /// use tako::types::Request;
    /// use std::sync::Arc;
    ///
    /// # async fn example() {
    /// # let middlewares = Arc::new(Vec::new());
    /// # let endpoint = Arc::new(|_req| Box::pin(async {
    /// #     tako::types::Response::new(tako::body::TakoBody::empty())
    /// # }) as std::pin::Pin<Box<dyn std::future::Future<Output = _> + Send>>);
    /// let next = Next {
    ///     middlewares,
    ///     endpoint,
    /// };
    ///
    /// let request = Request::builder().body(tako::body::TakoBody::empty()).unwrap();
    /// let response = next.run(request).await;
    /// # }
    /// ```
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
