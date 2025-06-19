use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use http::Method;
use regex::Regex;
use tokio::sync::RwLock;

use crate::{
    handler::BoxedHandler,
    types::{BoxedRequestFuture, Request},
};

pub struct Route {
    pub path: String,
    pub pattern: String,
    pub regex: Regex,
    pub param_names: Vec<String>,
    pub method: Method,
    pub handler: BoxedHandler,
    pub middlewares:
        RwLock<Vec<Box<dyn Fn(Request) -> BoxedRequestFuture + Send + Sync + 'static>>>,
    pub tsr: bool,
}

impl Route {
    pub fn new(path: String, method: Method, handler: BoxedHandler, tsr: Option<bool>) -> Self {
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

    pub fn middleware<F, Fut>(self: Arc<Self>, f: F) -> Arc<Self>
    where
        F: Fn(Request) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Request> + Send + 'static,
    {
        let this = self.clone();

        tokio::spawn(async move {
            let mut lock = this.middlewares.write().await;
            lock.push(Box::new(move |req: Request| -> BoxedRequestFuture {
                Box::pin(f(req))
            }));
        });

        self
    }

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
