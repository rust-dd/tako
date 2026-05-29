//! `application/problem+json` error normalizer middleware.
//!
//! Wraps the response so 4xx and 5xx replies that lack a JSON body are
//! rewritten into RFC 7807 / RFC 9457 problem documents. Handlers that
//! already produced a structured JSON error stay authoritative — the
//! `Content-Type` of the original response is the trigger.
//!
//! Sister to [`Router::use_problem_json`](tako_core::router::Router::use_problem_json)
//! / [`tako::problem::default_problem_responder`](tako_core::problem::default_problem_responder).
//! The router hook fires only when the response originated from the framework
//! itself (e.g. 404, 405, default error handler), whereas this middleware
//! converts any 4xx/5xx that bubbles up from handler code.

use std::future::Future;
use std::pin::Pin;

use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::problem::default_problem_responder;
use tako_core::types::Request;
use tako_core::types::Response;

/// Middleware that rewrites non-JSON 4xx/5xx responses into `problem+json`.
pub struct ProblemJson {
  /// Convert 4xx responses (default true).
  client_errors: bool,
  /// Convert 5xx responses (default true).
  server_errors: bool,
}

impl Default for ProblemJson {
  fn default() -> Self {
    Self::new()
  }
}

impl ProblemJson {
  /// Creates the middleware with both 4xx and 5xx conversion enabled.
  pub fn new() -> Self {
    Self {
      client_errors: true,
      server_errors: true,
    }
  }

  /// Toggles 4xx → problem+json conversion.
  pub fn client_errors(mut self, on: bool) -> Self {
    self.client_errors = on;
    self
  }

  /// Toggles 5xx → problem+json conversion.
  pub fn server_errors(mut self, on: bool) -> Self {
    self.server_errors = on;
    self
  }
}

impl IntoMiddleware for ProblemJson {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let client = self.client_errors;
    let server = self.server_errors;

    move |req: Request, next: Next| {
      Box::pin(async move {
        let resp = next.run(req).await;
        let status = resp.status();
        let should_convert =
          (client && status.is_client_error()) || (server && status.is_server_error());
        if !should_convert {
          return resp;
        }
        default_problem_responder(resp)
      })
    }
  }
}
