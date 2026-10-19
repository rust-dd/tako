//! Rewrites handlers' non-JSON 4xx/5xx responses into RFC 9457 problem documents; the router's
//! [`use_problem_json`](tako_rs_core::router::Router::use_problem_json) covers only its own.

use std::future::Future;
use std::pin::Pin;

use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::problem::default_problem_responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

pub struct ProblemJson {
  client_errors: bool,
  server_errors: bool,
}

impl Default for ProblemJson {
  fn default() -> Self {
    Self::new()
  }
}

impl ProblemJson {
  /// Converts both 4xx and 5xx responses.
  pub fn new() -> Self {
    Self {
      client_errors: true,
      server_errors: true,
    }
  }

  pub fn client_errors(mut self, on: bool) -> Self {
    self.client_errors = on;
    self
  }

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
