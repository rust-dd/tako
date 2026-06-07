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

/// Type alias for a global error handler function.
///
/// Called when a response has a server error status (5xx). Receives the original
/// response and can transform it (e.g., to return JSON errors instead of plain text).
pub type ErrorHandler = Arc<dyn Fn(Response) -> Response + Send + Sync + 'static>;

impl Router {
  /// Adds global middleware to the router.
  ///
  /// Global middleware is executed for all routes in the order it was added,
  /// before any route-specific middleware. Middleware can modify requests,
  /// generate responses, or perform side effects like logging or authentication.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, middleware::Next, types::Request};
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

    // RCU-style append: rebuild the Vec atomically against concurrent pushers.
    // ArcSwap retries the closure on CAS conflict, so concurrent middleware
    // registrations cannot lose entries.
    self.middlewares.rcu(move |current| {
      let mut next = Vec::with_capacity(current.len() + 1);
      next.extend(current.iter().cloned());
      next.push(mw.clone());
      Arc::new(next)
    });
    self.has_global_middleware.store(true, Ordering::Release);
    self
  }

  /// Sets a fallback handler that will be executed when no route matches.
  ///
  /// The fallback runs after global middlewares and can be used to implement
  /// custom 404 pages, catch-all logic, or method-independent handlers.
  ///
  /// # Examples
  ///
  /// ```rust
  /// use tako::{router::Router, Method, responder::Responder, types::Request};
  ///
  /// async fn not_found(_req: Request) -> impl Responder { "Not Found" }
  ///
  /// let mut router = Router::new();
  /// router.route(Method::GET, "/", |_req| async { "Hello" });
  /// router.fallback(not_found);
  /// ```
  pub fn fallback<F, Fut, R>(&mut self, handler: F) -> &mut Self
  where
    F: Fn(Request) -> Fut + Clone + Send + Sync + 'static,
    Fut: std::future::Future<Output = R> + Send + 'static,
    R: Responder + Send + 'static,
  {
    // Use the Request-arg handler impl to box the fallback
    self.fallback = Some(BoxHandler::new::<F, (Request,)>(handler));
    self
  }

  /// Sets a fallback handler that supports extractors (like `Path`, `Query`, etc.).
  ///
  /// Use this when your fallback needs to parse request data via extractors. If you
  /// only need access to the raw `Request`, prefer `fallback` for simpler type inference.
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

  /// Sets a default timeout for all routes.
  ///
  /// This timeout can be overridden on individual routes using `Route::timeout`.
  /// When a request exceeds the timeout duration, the timeout fallback handler
  /// is invoked (if configured) or a 408 Request Timeout response is returned.
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

  /// Sets a fallback handler that will be executed when a request times out.
  ///
  /// If no timeout fallback is set, a default 408 Request Timeout response is returned.
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

  /// Sets a global error handler for 5xx responses.
  ///
  /// The error handler receives any response with a server error status and can
  /// transform it (e.g., to return JSON-formatted errors instead of plain text).
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

  /// Sets a global error handler for 4xx responses.
  ///
  /// Mirrors [`Router::error_handler`] but fires for client errors. Useful for
  /// converting bare 404 / 405 / 422 responses into structured error documents
  /// (e.g. via [`crate::problem::default_problem_responder`]).
  pub fn client_error_handler(
    &mut self,
    handler: impl Fn(Response) -> Response + Send + Sync + 'static,
  ) -> &mut Self {
    self.client_error_handler = Some(Arc::new(handler));
    self
  }

  /// Convenience: install [`crate::problem::default_problem_responder`] for
  /// both 4xx and 5xx so unhandled errors always render as
  /// `application/problem+json`.
  pub fn use_problem_json(&mut self) -> &mut Self {
    let h: ErrorHandler = Arc::new(crate::problem::default_problem_responder);
    self.error_handler = Some(h.clone());
    self.client_error_handler = Some(h);
    self
  }
}
