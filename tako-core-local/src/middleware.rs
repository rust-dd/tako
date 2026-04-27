//! `!Send` middleware system.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use tako_core::types::Request;
use tako_core::types::Response;

use crate::handler::LocalBoxHandler;
use crate::handler::LocalBoxFuture;

/// Boxed `!Send` middleware function.
pub type LocalBoxMiddleware =
  Rc<dyn Fn(Request, LocalNext) -> LocalBoxFuture<'static, Response>>;

/// Trait for converting a function into a `LocalBoxMiddleware`.
pub trait LocalIntoMiddleware {
  fn into_local_middleware(
    self,
  ) -> impl Fn(Request, LocalNext) -> Pin<Box<dyn Future<Output = Response> + 'static>>
       + Clone
       + 'static;
}

/// Position in the `!Send` middleware chain.
pub struct LocalNext {
  pub global_middlewares: Rc<Vec<LocalBoxMiddleware>>,
  pub route_middlewares: Rc<Vec<LocalBoxMiddleware>>,
  pub index: usize,
  pub endpoint: LocalBoxHandler,
}

impl Clone for LocalNext {
  fn clone(&self) -> Self {
    Self {
      global_middlewares: Rc::clone(&self.global_middlewares),
      route_middlewares: Rc::clone(&self.route_middlewares),
      index: self.index,
      endpoint: self.endpoint.clone(),
    }
  }
}

impl LocalNext {
  pub async fn run(mut self, req: Request) -> Response {
    let mw = if let Some(mw) = self.global_middlewares.get(self.index) {
      Some(mw.clone())
    } else {
      self
        .route_middlewares
        .get(self.index.saturating_sub(self.global_middlewares.len()))
        .cloned()
    };

    if let Some(mw) = mw {
      self.index += 1;
      mw(req, self).await
    } else {
      self.endpoint.call(req).await
    }
  }
}
