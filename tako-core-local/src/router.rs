//! `!Send` router.
//!
//! Minimal but functional mirror of [`tako_core::router::Router`]: route
//! registration with method + path, global middleware, dispatch through the
//! middleware chain to the matched handler. Plugin/signals/timeout/openapi
//! infrastructure from the thread-safe router is intentionally omitted —
//! add it incrementally as needed.

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use http::Method;
use http::StatusCode;
use smallvec::SmallVec;

use tako_core::body::TakoBody;
use tako_core::extractors::params::PathParams;
use tako_core::types::Request;
use tako_core::types::Response;

use crate::handler::LocalBoxHandler;
use crate::handler::LocalBoxFuture;
use crate::handler::LocalHandler;
use crate::middleware::LocalBoxMiddleware;
use crate::middleware::LocalNext;
use crate::route::LocalRoute;

/// Thread-local HTTP router. Single-threaded by design.
pub struct LocalRouter {
  routes: RefCell<MethodMap>,
  middlewares: RefCell<Rc<Vec<LocalBoxMiddleware>>>,
}

#[derive(Default)]
struct MethodMap {
  get: matchit::Router<Rc<LocalRoute>>,
  post: matchit::Router<Rc<LocalRoute>>,
  put: matchit::Router<Rc<LocalRoute>>,
  delete: matchit::Router<Rc<LocalRoute>>,
  patch: matchit::Router<Rc<LocalRoute>>,
  head: matchit::Router<Rc<LocalRoute>>,
  options: matchit::Router<Rc<LocalRoute>>,
  trace: matchit::Router<Rc<LocalRoute>>,
  connect: matchit::Router<Rc<LocalRoute>>,
}

impl MethodMap {
  fn slot(&mut self, method: &Method) -> &mut matchit::Router<Rc<LocalRoute>> {
    match *method {
      Method::GET => &mut self.get,
      Method::POST => &mut self.post,
      Method::PUT => &mut self.put,
      Method::DELETE => &mut self.delete,
      Method::PATCH => &mut self.patch,
      Method::HEAD => &mut self.head,
      Method::OPTIONS => &mut self.options,
      Method::TRACE => &mut self.trace,
      Method::CONNECT => &mut self.connect,
      _ => &mut self.get,
    }
  }

  fn lookup(&self, method: &Method) -> &matchit::Router<Rc<LocalRoute>> {
    match *method {
      Method::GET => &self.get,
      Method::POST => &self.post,
      Method::PUT => &self.put,
      Method::DELETE => &self.delete,
      Method::PATCH => &self.patch,
      Method::HEAD => &self.head,
      Method::OPTIONS => &self.options,
      Method::TRACE => &self.trace,
      Method::CONNECT => &self.connect,
      _ => &self.get,
    }
  }
}

impl Default for LocalRouter {
  fn default() -> Self {
    Self::new()
  }
}

impl LocalRouter {
  pub fn new() -> Self {
    Self {
      routes: RefCell::new(MethodMap::default()),
      middlewares: RefCell::new(Rc::new(Vec::new())),
    }
  }

  /// Registers a route with the given method, path and handler.
  pub fn route<H, T>(&self, method: Method, path: &str, handler: H) -> Rc<LocalRoute>
  where
    H: LocalHandler<T> + Clone,
  {
    let route = Rc::new(LocalRoute::new(
      method.clone(),
      path.to_string(),
      LocalBoxHandler::new::<H, T>(handler),
    ));

    if let Err(err) = self
      .routes
      .borrow_mut()
      .slot(&method)
      .insert(path.to_string(), Rc::clone(&route))
    {
      panic!("LocalRouter: failed to register {method} {path}: {err}");
    }
    route
  }

  /// Adds a global middleware that runs on every request before any
  /// route-scoped middleware.
  pub fn middleware<F, Fut>(&self, mw: F)
  where
    F: Fn(Request, LocalNext) -> Fut + Clone + 'static,
    Fut: std::future::Future<Output = Response> + 'static,
  {
    let mw: LocalBoxMiddleware = Rc::new(move |req, next| -> LocalBoxFuture<'static, Response> {
      let mw = mw.clone();
      Box::pin(mw(req, next))
    });

    let mut current = self.middlewares.borrow_mut();
    let mut new = (**current).clone();
    new.push(mw);
    *current = Rc::new(new);
  }

  /// Dispatches an incoming request. Returns a 404 if no route matches.
  pub fn dispatch(
    self: &Rc<Self>,
    mut req: Request,
  ) -> Pin<Box<dyn std::future::Future<Output = Response> + '_>> {
    let routes = self.routes.borrow();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let global_mw = Rc::clone(&self.middlewares.borrow());

    let matched = routes.lookup(&method).at(&path);
    let (route, params) = match matched {
      Ok(m) => {
        let route = Rc::clone(m.value);
        let mut it = m.params.iter();
        let first = it.next();
        let params = first.map(|(fk, fv)| {
          let mut p = SmallVec::<[(String, String); 4]>::new();
          p.push((fk.to_string(), fv.to_string()));
          for (k, v) in it {
            p.push((k.to_string(), v.to_string()));
          }
          PathParams(p)
        });
        (route, params)
      }
      Err(_) => {
        let resp = http::Response::builder()
          .status(StatusCode::NOT_FOUND)
          .body(TakoBody::empty())
          .expect("valid 404 response");
        return Box::pin(async move { resp });
      }
    };
    drop(routes);

    if let Some(p) = params {
      req.extensions_mut().insert(p);
    }

    let route_mw: Rc<Vec<LocalBoxMiddleware>> =
      Rc::new(route.middlewares.borrow().clone());
    let endpoint = route.handler.clone();

    let next = LocalNext {
      global_middlewares: global_mw,
      route_middlewares: route_mw,
      index: 0,
      endpoint,
    };

    Box::pin(async move { next.run(req).await })
  }
}
