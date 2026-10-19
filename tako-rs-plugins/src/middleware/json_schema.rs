//! Validates JSON request or response bodies against a schema compiled once; violations become
//! `application/problem+json`. Bodies are buffered, so mount it only where needed.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ValidateTarget {
  Request,
  Response,
}

pub struct JsonSchema {
  validator: Arc<Validator>,
  target: ValidateTarget,
  max_bytes: usize,
}

// Cold startup constructors; boxing the error would only burden callers.
#[allow(clippy::result_large_err)]
impl JsonSchema {
  /// Invalid or non-conforming JSON bodies get 400 before the handler runs.
  pub fn for_request(schema: Value) -> Result<Self, jsonschema::ValidationError<'static>> {
    Self::new(schema, ValidateTarget::Request)
  }

  /// Non-conforming JSON responses become 500; unparsable ones pass through unchanged.
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

  /// Defaults to 1 MiB; larger requests get 413 and larger responses become 500.
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
                // Partially consumed, so the original body cannot be replayed.
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
