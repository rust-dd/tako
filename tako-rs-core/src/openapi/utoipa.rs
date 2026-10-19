//! `utoipa` integration: derive the spec with `#[derive(OpenApi)]` and serve it with
//! [`OpenApiJson`].

use http::StatusCode;
pub use utoipa::Modify;
pub use utoipa::OpenApi;
pub use utoipa::ToResponse;
pub use utoipa::ToSchema;
pub use utoipa::openapi;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Response;

/// Serves the spec as JSON.
///
/// # Examples
///
/// ```rust,ignore
/// use tako::openapi::utoipa::{OpenApi, OpenApiJson};
///
/// #[derive(OpenApi)]
/// #[openapi(paths(health), components(schemas(HealthResponse)))]
/// struct ApiDoc;
///
/// async fn openapi_handler(_req: tako::types::Request) -> OpenApiJson {
///     OpenApiJson(ApiDoc::openapi())
/// }
/// ```
pub struct OpenApiJson(pub openapi::OpenApi);

impl Responder for OpenApiJson {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          http::HeaderValue::from_static("application/json"),
        );
        res
      }
      Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
  }
}

/// Serves the spec as YAML; needs the `utoipa-yaml` feature.
#[cfg(feature = "utoipa-yaml")]
pub struct OpenApiYaml(pub openapi::OpenApi);

#[cfg(feature = "utoipa-yaml")]
impl Responder for OpenApiYaml {
  fn into_response(self) -> Response {
    match self.0.to_yaml() {
      Ok(yaml) => {
        let mut res = Response::new(TakoBody::from(yaml));
        res.headers_mut().insert(
          http::header::CONTENT_TYPE,
          http::HeaderValue::from_static("application/x-yaml"),
        );
        res
      }
      Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
  }
}
