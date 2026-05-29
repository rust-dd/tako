//! JSON-schema request / response validator middleware.
//!
//! Builds a [`jsonschema::Validator`] once at construction and applies it to
//! the request body (or response body) of every JSON request. Non-JSON
//! content types are passed through unchanged. Validation failures emit a
//! `application/problem+json` response listing the offending paths.
//!
//! The middleware buffers the body for inspection. For large streaming
//! payloads, attach this only on the routes that actually need validation —
//! the design favors correctness over zero-copy throughput on the hot path.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderValue;
use http::StatusCode;
use http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use jsonschema::Validator;
use serde_json::Value;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

/// What the middleware should validate.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidateTarget {
  /// Validate the request body before invoking the handler.
  Request,
  /// Validate the response body before returning it.
  Response,
}

/// JSON-schema validator middleware.
pub struct JsonSchema {
  validator: Arc<Validator>,
  target: ValidateTarget,
  max_bytes: usize,
}

// `jsonschema::ValidationError<'static>` is a few hundred bytes; boxing it on
// every constructor would be churn for callers. The constructors are cold-path
// startup code, so size on the error variant doesn't matter.
#[allow(clippy::result_large_err)]
impl JsonSchema {
  /// Builds a validator that runs against the request body.
  pub fn for_request(schema: Value) -> Result<Self, jsonschema::ValidationError<'static>> {
    Self::new(schema, ValidateTarget::Request)
  }

  /// Builds a validator that runs against the response body.
  pub fn for_response(schema: Value) -> Result<Self, jsonschema::ValidationError<'static>> {
    Self::new(schema, ValidateTarget::Response)
  }

  fn new(
    schema: Value,
    target: ValidateTarget,
  ) -> Result<Self, jsonschema::ValidationError<'static>> {
    let validator = jsonschema::validator_for(&schema)?;
    Ok(Self {
      validator: Arc::new(validator),
      target,
      max_bytes: 1024 * 1024,
    })
  }

  /// Maximum body size the middleware is willing to validate. Larger payloads
  /// are rejected with `413` (request side) or passed through (response side).
  pub fn max_bytes(mut self, n: usize) -> Self {
    self.max_bytes = n;
    self
  }
}

fn is_json(content_type: Option<&HeaderValue>) -> bool {
  content_type
    .and_then(|v| v.to_str().ok())
    .map(str::to_ascii_lowercase)
    .is_some_and(|s| s.contains("json"))
}

fn problem(status: StatusCode, errors: &[String]) -> Response {
  let body = serde_json::json!({
    "type": "about:blank",
    "title": status.canonical_reason().unwrap_or("Bad Request"),
    "status": status.as_u16(),
    "errors": errors,
  });
  let mut resp = http::Response::builder()
    .status(status)
    .body(TakoBody::from(
      serde_json::to_vec(&body).unwrap_or_default(),
    ))
    .expect("valid problem response");
  resp.headers_mut().insert(
    CONTENT_TYPE,
    HeaderValue::from_static("application/problem+json"),
  );
  resp
}

impl IntoMiddleware for JsonSchema {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let validator = self.validator;
    let target = self.target;
    let max_bytes = self.max_bytes;

    move |req: Request, next: Next| {
      let validator = validator.clone();
      Box::pin(async move {
        match target {
          ValidateTarget::Request => {
            if !is_json(req.headers().get(CONTENT_TYPE)) {
              return next.run(req).await;
            }
            let (parts, body) = req.into_parts();
            let limited = http_body_util::Limited::new(body, max_bytes);
            let collected = match limited.collect().await {
              Ok(c) => c.to_bytes(),
              Err(_) => {
                return http::Response::builder()
                  .status(StatusCode::PAYLOAD_TOO_LARGE)
                  .body(TakoBody::empty())
                  .expect("valid 413");
              }
            };
            match serde_json::from_slice::<Value>(&collected) {
              Ok(value) => {
                let errors: Vec<String> = validator
                  .iter_errors(&value)
                  .map(|e| e.to_string())
                  .collect();
                if !errors.is_empty() {
                  return problem(StatusCode::BAD_REQUEST, &errors);
                }
                let new_req = http::Request::from_parts(parts, TakoBody::from(collected));
                next.run(new_req).await
              }
              Err(e) => problem(StatusCode::BAD_REQUEST, &[e.to_string()]),
            }
          }
          ValidateTarget::Response => {
            let resp = next.run(req).await;
            if !is_json(resp.headers().get(CONTENT_TYPE)) {
              return resp;
            }
            let (parts, body) = resp.into_parts();
            let limited = http_body_util::Limited::new(body, max_bytes);
            let collected = match limited.collect().await {
              Ok(c) => c.to_bytes(),
              Err(_) => {
                // Response exceeded the validator budget — surface as 500;
                // the upstream body has been partially consumed.
                return http::Response::builder()
                  .status(StatusCode::INTERNAL_SERVER_ERROR)
                  .body(TakoBody::empty())
                  .expect("valid 500");
              }
            };
            match serde_json::from_slice::<Value>(&collected) {
              Ok(value) => {
                let errors: Vec<String> = validator
                  .iter_errors(&value)
                  .map(|e| e.to_string())
                  .collect();
                if !errors.is_empty() {
                  return problem(StatusCode::INTERNAL_SERVER_ERROR, &errors);
                }
                http::Response::from_parts(parts, TakoBody::from(collected))
              }
              Err(_) => http::Response::from_parts(parts, TakoBody::from(collected)),
            }
          }
        }
      })
    }
  }
}
