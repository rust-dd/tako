//! HTTP route definition and path matching functionality.
//!
//! This module provides the core `Route` struct for defining HTTP routes with path patterns,
//! parameter extraction, and middleware support. Routes can contain dynamic segments like
//! `{id}` that are captured as parameters, and support method-specific handlers with
//! optional trailing slash redirection and route-specific middleware chains.
//!
//! # Examples
//!
//! ```rust
//! use tako::route::Route;
//! use tako::handler::BoxHandler;
//! use tako::types::Request;
//! use http::Method;
//!
//! async fn handler(_req: Request) -> &'static str {
//!     "Hello, World!"
//! }
//!
//! let route = Route::new(
//!     "/users/{id}".to_string(),
//!     Method::GET,
//!     BoxHandler::new(handler),
//!     None
//! );
//!
//! let params = route.match_path("/users/123").unwrap();
//! assert_eq!(params.get("id"), Some(&"123".to_string()));
//! ```

use std::{
    collections::VecDeque,
    sync::{Arc, RwLock},
};

use http::Method;

use crate::{
    handler::BoxHandler,
    middleware::Next,
    responder::Responder,
    types::{BoxMiddleware, Request},
};

/// HTTP route with path pattern matching and middleware support.
pub struct Route {
    /// Original path string used to create this route.
    pub path: String,
    /// HTTP method this route responds to.
    pub method: Method,
    /// Handler function to execute when route is matched.
    pub handler: BoxHandler,
    /// Route-specific middleware chain.
    pub middlewares: RwLock<VecDeque<BoxMiddleware>>,
    /// Whether trailing slash redirection is enabled.
    pub tsr: bool,
}

impl Route {
    /// Creates a new route with the specified path, method, and handler.
    pub fn new(path: String, method: Method, handler: BoxHandler, tsr: Option<bool>) -> Self {
        Self {
            path,
            method,
            handler,
            middlewares: RwLock::new(VecDeque::new()),
            tsr: tsr.unwrap_or(false),
        }
    }

    /// Adds middleware to this route's execution chain.
    pub fn middleware<F, Fut, R>(&self, f: F) -> &Self
    where
        F: Fn(Request, Next) -> Fut + Clone + Send + Sync + 'static,
        Fut: std::future::Future<Output = R> + Send + 'static,
        R: Responder + Send + 'static,
    {
        let mw: BoxMiddleware = Arc::new(move |req, next| {
            let fut = f(req, next); // Fut<'a>

            Box::pin(async move { fut.await.into_response() })
        });

        self.middlewares.write().unwrap().push_back(mw);
        self
    }
}
