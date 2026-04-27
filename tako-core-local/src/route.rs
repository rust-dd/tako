//! `!Send` route definition.

use std::cell::RefCell;

use http::Method;

use crate::handler::LocalBoxHandler;
use crate::middleware::LocalBoxMiddleware;

/// A registered route together with any per-route middleware.
pub struct LocalRoute {
  pub method: Method,
  pub path: String,
  pub handler: LocalBoxHandler,
  pub(crate) middlewares: RefCell<Vec<LocalBoxMiddleware>>,
}

impl LocalRoute {
  pub fn new(method: Method, path: String, handler: LocalBoxHandler) -> Self {
    Self {
      method,
      path,
      handler,
      middlewares: RefCell::new(Vec::new()),
    }
  }

  /// Adds a route-scoped middleware that runs after global middleware on this route.
  pub fn middleware(&self, mw: LocalBoxMiddleware) {
    self.middlewares.borrow_mut().push(mw);
  }
}
