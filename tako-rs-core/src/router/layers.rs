//! Global middleware, fallbacks, timeouts, and error-handler wiring.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use super::Router;
use crate::handler::BoxHandler;
use crate::handler::Handler;
use crate::middleware::Next;
use crate::responder::Responder;
use crate::types::BoxMiddleware;
use crate::types::Request;
use crate::types::Response;

/// Response rewriter set by [`Router::error_handler`] and [`Router::client_error_handler`].
pub type ErrorHandler = Arc<dyn Fn(Response) -> Response + Send + Sync + 'static>;

/// Error formatter with access to request metadata, invoked for 4xx and 5xx.
pub type ErrorHandlerWithParts =
  Arc<dyn Fn(&http::request::Parts, Response) -> Response + Send + Sync>;

impl Router {
  /// Installs a request-aware formatter for client and server error responses.
  /// Request metadata is cloned only when this handler is configured.
  pub fn error_handler_with_parts(
    &mut self,
    handler: impl Fn(&http::request::Parts, Response) -> Response + Send + Sync + 'static,
  ) -> &mut Self {
    self.error_handler_with_parts = Some(Arc::new(handler));
    self
  }

  /// Sets the maximum bytes buffered by body extractors (default: 2 MiB).
  pub fn body_limit(&mut self, bytes: usize) -> &mut Self {
    self.body_limit = Some(crate::extractors::body::BodyLimit(Some(bytes)));
    self
  }

  /// Explicitly allows body extractors to buffer without a size limit.
  pub fn disable_body_limit(&mut self) -> &mut Self {
    self.body_limit = Some(crate::extractors::body::BodyLimit(None));
    self
  }

  /// Status for timed-out requests when no [`Router::timeout_fallback`] is set (default: 504).
  pub fn timeout_status(&mut self, status: http::StatusCode) -> &mut Self {
    self.timeout_status = status;
    self
  }

  /// Adds middleware with a mutable receiver for chaining builder methods.
  pub fn layer<F, Fut, R>(&mut self, f: F) -> &mut Self
  where
    F: Fn(Request, Next) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    self.middleware(f);
    self
  }

  /// Adds middleware that runs, in registration order, before every route's own middleware.
  /// Takes `&self` so plugins can register it during setup; [`Router::layer`] chains instead.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, middleware::Next, types::Request};
  /// use tako::responder::Responder;
  ///
  /// let mut router = Router::new();
  ///
  /// // Logging middleware
  /// router.middleware(|req, next| async move {
  ///     println!("Request: {} {}", req.method(), req.uri());
  ///     let response = next.run(req).await;
  ///     println!("Response: {}", response.status());
  ///     response
  /// });
  ///
  /// // Authentication middleware
  /// router.middleware(|req, next| async move {
  ///     if req.headers().contains_key("authorization") {
  ///         next.run(req).await
  ///     } else {
  ///         "Unauthorized".into_response()
  ///     }
  /// });
  /// ```
  pub fn middleware<F, Fut, R>(&self, f: F) -> &Self
  where
    F: Fn(Request, Next) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    let mw: BoxMiddleware = Arc::new(move |req, next| {
      let fut = f(req, next);
      Box::pin(async move { fut.await.into_response() })
    });

    // `rcu` retries on a concurrent update, so no registration is lost.
    self.middlewares.rcu(move |current| {
      let mut next = Vec::with_capacity(current.len() + 1);
      next.extend(current.iter().cloned());
      next.push(mw.clone());
      Arc::new(next)
    });
    self.has_global_middleware.store(true, Ordering::Release);
    self
  }

  /// Handles requests whose path matches no route, behind the global middleware.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn not_found(_req: Request) -> impl Responder { "Not Found" }
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/", || async { "Hello" });
  /// router.fallback(not_found);
  /// ```
  pub fn fallback<F, Fut, R>(&mut self, handler: F) -> &mut Self
  where
    F: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    self.fallback = Some(BoxHandler::new::<F, (Request,)>(handler));
    self
  }

  /// [`Router::fallback`] for handlers that take extractors; `fallback` infers types more easily.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, responder::Responder, extractors::{path::Path, query::Query}};
  ///
  /// #[derive(serde::Deserialize)]
  /// struct Q { q: Option<String> }
  ///
  /// async fn fallback_with_q(Path(_p): Path<String>, Query(_q): Query<Q>) -> impl Responder {
  ///     "Not Found"
  /// }
  ///
  /// let mut router = Router::new();
  /// router.fallback_with_extractors(fallback_with_q);
  /// ```
  pub fn fallback_with_extractors<H, T>(&mut self, handler: H) -> &mut Self
  where
    H: Handler<T> + Clone + 'static,
  {
    self.fallback = Some(BoxHandler::new::<H, T>(handler));
    self
  }

  /// Timeout for routes without their own [`Route::timeout`](crate::route::Route::timeout);
  /// late requests get the [`Router::timeout_fallback`] or [`Router::timeout_status`].
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use std::time::Duration;
  ///
  /// let mut router = Router::new();
  /// router.timeout(Duration::from_secs(30));
  /// ```
  pub fn timeout(&mut self, duration: Duration) -> &mut Self {
    self.timeout = Some(duration);
    self
  }

  /// Answers timed-out requests; the handler gets a copy of the request without its body.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, responder::Responder, types::Request};
  /// use std::time::Duration;
  ///
  /// async fn timeout_handler(_req: Request) -> impl Responder {
  ///     "Request took too long"
  /// }
  ///
  /// let mut router = Router::new();
  /// router.timeout(Duration::from_secs(30));
  /// router.timeout_fallback(timeout_handler);
  /// ```
  pub fn timeout_fallback<F, Fut, R>(&mut self, handler: F) -> &mut Self
  where
    F: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    self.timeout_fallback = Some(BoxHandler::new::<F, (Request,)>(handler));
    self
  }

  /// Rewrites every 5xx response, for example into JSON.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::router::Router;
  /// use tako::body::TakoBody;
  ///
  /// let mut router = Router::new();
  /// router.error_handler(|resp| {
  ///     let status = resp.status();
  ///     let body = format!(r#"{{"error": "{}"}}"#, status.canonical_reason().unwrap_or("Unknown"));
  ///     let mut res = http::Response::new(TakoBody::from(body));
  ///     *res.status_mut() = status;
  ///     res.headers_mut().insert(
  ///         http::header::CONTENT_TYPE,
  ///         http::HeaderValue::from_static("application/json"),
  ///     );
  ///     res
  /// });
  /// ```
  pub fn error_handler(
    &mut self,
    handler: impl Fn(Response) -> Response + Send + Sync + 'static,
  ) -> &mut Self {
    self.error_handler = Some(Arc::new(handler));
    self
  }

  /// Rewrites every 4xx response, as [`Router::error_handler`] does for 5xx.
  pub fn client_error_handler(
    &mut self,
    handler: impl Fn(Response) -> Response + Send + Sync + 'static,
  ) -> &mut Self {
    self.client_error_handler = Some(Arc::new(handler));
    self
  }

  /// Installs [`default_problem_responder`](crate::problem::default_problem_responder) as both
  /// the 4xx and the 5xx handler.
  pub fn use_problem_json(&mut self) -> &mut Self {
    let h: ErrorHandler = Arc::new(crate::problem::default_problem_responder);
    self.error_handler = Some(h.clone());
    self.client_error_handler = Some(h);
    self
  }
}
