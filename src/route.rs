/// This module defines the `Route` struct and its associated methods for managing HTTP routes.
///
/// The `Route` struct encapsulates information about a specific route, including its path,
/// HTTP method, handler, and any associated middleware. It also provides utilities for
/// matching paths and extracting parameters from dynamic segments.
use std::{
    collections::HashMap,
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

/// Represents an HTTP route with its associated path, method, handler, and middleware.
///
/// The `Route` struct is responsible for storing route-specific information and provides
/// methods for matching paths and managing middleware.
///
/// # Fields
/// - `path`: The original path string for the route.
/// - `pattern`: The pattern used for matching the route.
/// - `regex`: A compiled regular expression for matching the route.
/// - `param_names`: A list of parameter names extracted from the route pattern.
/// - `method`: The HTTP method associated with the route.
/// - `handler`: The handler function to be executed when the route is matched.
/// - `middlewares`: A list of middleware functions to be executed before the handler.
/// - `tsr`: A flag indicating whether trailing slash redirection is enabled.
pub struct Route {
    pub path: String,
    pub pattern: String,
    pub regex: Regex,
    pub param_names: Vec<String>,
    pub method: Method,
    pub handler: BoxHandler,
    pub middlewares: RwLock<Vec<BoxMiddleware>>,
    pub tsr: bool,
}

impl Route {
    /// Creates a new `Route` instance.
    ///
    /// # Arguments
    /// - `path`: The path string for the route.
    /// - `method`: The HTTP method associated with the route.
    /// - `handler`: The handler function to be executed when the route is matched.
    /// - `tsr`: An optional flag indicating whether trailing slash redirection is enabled.
    ///
    /// # Returns
    /// A new `Route` instance.
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
            middlewares: RwLock::new(Vec::new()),
            tsr: tsr.unwrap_or(false),
        }
    }

    /// Adds a middleware function to the route.
    ///
    /// Middleware functions are executed before the route's handler and can modify
    /// the request or return a response directly.
    ///
    /// # Arguments
    /// - `f`: A middleware function that takes a `Request` and returns a `Future`.
    ///
    /// # Returns
    /// An `Arc` pointing to the updated `Route` instance.
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

        self.middlewares.write().unwrap().push(mw);
        self
    }

    /// Matches a given path against the route's pattern.
    ///
    /// If the path matches, this method extracts and returns the dynamic parameters
    /// as a `HashMap`. If the path does not match, it returns `None`.
    ///
    /// # Arguments
    /// - `path`: The path string to match against the route's pattern.
    ///
    /// # Returns
    /// An `Option` containing a `HashMap` of parameter names and values if the path matches,
    /// or `None` if it does not.
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

    /// Parses a route pattern into a regular expression and extracts parameter names.
    ///
    /// This method converts a route pattern (e.g., `/users/{id}`) into a regular expression
    /// for matching paths and extracts the names of dynamic parameters (e.g., `id`).
    ///
    /// # Arguments
    /// - `pattern`: The route pattern string to parse.
    ///
    /// # Returns
    /// A tuple containing:
    /// - A `Regex` for matching paths.
    /// - A `Vec<String>` of parameter names.
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
