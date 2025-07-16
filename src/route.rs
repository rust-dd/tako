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
    collections::{HashMap, VecDeque},
    sync::{Arc, RwLock},
};

use http::Method;
use regex::Regex;

use crate::{
    handler::BoxHandler,
    middleware::Next,
    responder::Responder,
    types::{BoxMiddleware, Request},
};

/// HTTP route with path pattern matching and middleware support.
///
/// A `Route` represents a single HTTP endpoint with its associated path pattern,
/// HTTP method, handler function, and middleware chain. It provides pattern matching
/// capabilities for extracting parameters from dynamic path segments and manages
/// the execution flow through middleware to the final handler.
pub struct Route {
    /// Original path string used to create this route.
    pub path: String,
    /// Pattern string used for matching (currently same as path).
    pub pattern: String,
    /// Compiled regular expression for efficient path matching.
    pub regex: Regex,
    /// Names of parameters extracted from dynamic path segments.
    pub param_names: Vec<String>,
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
    ///
    /// The path pattern supports dynamic segments using curly braces (e.g., `/users/{id}`).
    /// These segments are captured as named parameters that can be extracted during
    /// request processing.
    ///
    /// # Arguments
    ///
    /// * `path` - Path pattern with optional dynamic segments like `/users/{id}`
    /// * `method` - HTTP method this route responds to
    /// * `handler` - Handler function to execute when route matches
    /// * `tsr` - Optional trailing slash redirection flag
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::route::Route;
    /// use tako::handler::BoxHandler;
    /// use tako::types::Request;
    /// use http::Method;
    ///
    /// async fn user_handler(_req: Request) -> &'static str {
    ///     "User page"
    /// }
    ///
    /// let route = Route::new(
    ///     "/users/{id}/posts/{post_id}".to_string(),
    ///     Method::GET,
    ///     BoxHandler::new(user_handler),
    ///     Some(true)
    /// );
    ///
    /// assert_eq!(route.param_names, vec!["id", "post_id"]);
    /// assert_eq!(route.method, Method::GET);
    /// assert!(route.tsr);
    /// ```
    pub fn new(path: String, method: Method, handler: BoxHandler, tsr: Option<bool>) -> Self {
        let pattern = path.clone();
        let (regex, param_names) = Self::parse_pattern(&pattern);

        Self {
            path,
            pattern,
            regex,
            param_names,
            method,
            handler,
            middlewares: RwLock::new(VecDeque::new()),
            tsr: tsr.unwrap_or(false),
        }
    }

    /// Adds middleware to this route's execution chain.
    ///
    /// Middleware functions are executed in the order they are added, before the
    /// route's handler. Each middleware can modify the request, return an early
    /// response, or pass control to the next middleware in the chain.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::route::Route;
    /// use tako::handler::BoxHandler;
    /// use tako::middleware::Next;
    /// use tako::types::Request;
    /// use http::Method;
    ///
    /// async fn handler(_req: Request) -> &'static str {
    ///     "Hello"
    /// }
    ///
    /// async fn auth_middleware(req: Request, next: Next) -> &'static str {
    ///     // Authentication logic here
    ///     "Authenticated response"
    /// }
    ///
    /// let route = Route::new(
    ///     "/protected".to_string(),
    ///     Method::GET,
    ///     BoxHandler::new(handler),
    ///     None
    /// );
    ///
    /// route.middleware(auth_middleware);
    /// ```
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

    /// Matches the given path against this route's pattern and extracts parameters.
    ///
    /// If the path matches the route's pattern, returns a HashMap containing the
    /// extracted parameter names and values. Dynamic segments in the route pattern
    /// (like `{id}`) are captured and made available as parameters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::route::Route;
    /// use tako::handler::BoxHandler;
    /// use tako::types::Request;
    /// use http::Method;
    ///
    /// async fn handler(_req: Request) -> &'static str {
    ///     "Handler"
    /// }
    ///
    /// let route = Route::new(
    ///     "/users/{id}/posts/{post_id}".to_string(),
    ///     Method::GET,
    ///     BoxHandler::new(handler),
    ///     None
    /// );
    ///
    /// let params = route.match_path("/users/123/posts/456").unwrap();
    /// assert_eq!(params.get("id"), Some(&"123".to_string()));
    /// assert_eq!(params.get("post_id"), Some(&"456".to_string()));
    ///
    /// assert!(route.match_path("/users/123").is_none());
    /// assert!(route.match_path("/posts/456").is_none());
    /// ```
    pub fn match_path(&self, path: &str) -> Option<HashMap<String, String>> {
        self.regex.captures(path).map(|caps| {
            self.param_names
                .iter()
                .enumerate()
                .filter_map(|(i, name)| {
                    caps.get(i + 1)
                        .map(|m| (name.clone(), m.as_str().to_string()))
                })
                .collect::<_>()
        })
    }

    /// Parses a route pattern into a regex and extracts parameter names.
    ///
    /// Converts route patterns with dynamic segments (e.g., `/users/{id}`) into
    /// regular expressions for efficient matching. Dynamic segments are replaced
    /// with capture groups, and their names are extracted for parameter mapping.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::route::Route;
    ///
    /// let (regex, params) = Route::parse_pattern("/users/{id}/posts/{post_id}");
    /// assert_eq!(params, vec!["id", "post_id"]);
    /// assert!(regex.is_match("/users/123/posts/456"));
    /// assert!(!regex.is_match("/users/123"));
    /// ```
    fn parse_pattern(pattern: &str) -> (Regex, Vec<String>) {
        let mut regex_str = String::from("^");
        let mut param_names = Vec::new();

        for s in pattern.trim_matches('/').split('/') {
            regex_str.push('/');

            if s.starts_with('{') && s.ends_with('}') {
                let param = &s[1..s.len() - 1];
                regex_str.push_str("([^/]+)");
                param_names.push(param.to_string());
            } else {
                regex_str.push_str(&regex::escape(s));
            }
        }

        regex_str.push('$');
        let regex = Regex::new(&regex_str).expect("Invalid route pattern");
        (regex, param_names)
    }
}
