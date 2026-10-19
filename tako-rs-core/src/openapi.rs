//! `OpenAPI` route metadata plus two spec backends: `utoipa` (derive macros, recommended) and
//! `vespera` (a spec built at runtime, also from the routes' metadata).
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::{router::Router, Method};
//!
//! let mut router = Router::new();
//! router.route(Method::GET, "/users/{id}", get_user)
//!     .summary("Get user by ID")
//!     .description("Retrieves a user by their unique identifier")
//!     .tag("users")
//!     .response(200, "Successful response")
//!     .response(404, "User not found");
//! ```
//!
//! ```rust,ignore
//! use tako::openapi::utoipa::{OpenApi, OpenApiJson, ToSchema};
//!
//! #[derive(ToSchema)]
//! struct User {
//!     id: u64,
//!     name: String,
//! }
//!
//! #[derive(OpenApi)]
//! #[openapi(components(schemas(User)))]
//! struct ApiDoc;
//!
//! async fn openapi(_req: tako::types::Request) -> OpenApiJson {
//!     OpenApiJson(ApiDoc::openapi())
//! }
//! ```
//!
//! ```rust,ignore
//! use tako::openapi::vespera::{OpenApi, Info, VesperaOpenApiJson};
//!
//! async fn openapi(_req: tako::types::Request) -> VesperaOpenApiJson {
//!     let spec = OpenApi {
//!         info: Info {
//!             title: "My API".to_string(),
//!             version: "1.0.0".to_string(),
//!             ..Default::default()
//!         },
//!         ..Default::default()
//!     };
//!     VesperaOpenApiJson(spec)
//! }
//! ```

use std::collections::BTreeMap;

/// `OpenAPI` operation metadata of one route.
#[derive(Clone, Debug, Default)]
pub struct RouteOpenApi {
  pub operation_id: Option<String>,
  pub summary: Option<String>,
  pub description: Option<String>,
  pub tags: Vec<String>,
  pub deprecated: bool,
  /// Response description per status code.
  pub responses: BTreeMap<u16, String>,
  pub parameters: Vec<OpenApiParameter>,
  pub request_body: Option<OpenApiRequestBody>,
  /// Names of the security schemes the operation requires.
  pub security: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct OpenApiParameter {
  pub name: String,
  pub location: ParameterLocation,
  pub description: Option<String>,
  pub required: bool,
}

#[derive(Clone, Debug, Default)]
pub enum ParameterLocation {
  #[default]
  Query,
  Header,
  Path,
  Cookie,
}

#[derive(Clone, Debug, Default)]
pub struct OpenApiRequestBody {
  pub description: Option<String>,
  pub required: bool,
  /// Media type such as `application/json`.
  pub content_type: String,
  /// Top-level properties of the body's object schema.
  pub schema_properties: Vec<RequestBodyProperty>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestBodyProperty {
  pub name: String,
  /// JSON Schema type such as `string`, `integer` or `boolean`.
  pub property_type: String,
  pub description: Option<String>,
}

#[cfg(feature = "utoipa")]
#[cfg_attr(docsrs, doc(cfg(feature = "utoipa")))]
pub mod utoipa;

#[cfg(feature = "vespera")]
#[cfg_attr(docsrs, doc(cfg(feature = "vespera")))]
pub mod vespera;

pub mod ui;
