//! OpenAPI documentation generation integrations.
//!
//! This module provides integrations with popular OpenAPI documentation generators:
//!
//! - **utoipa**: Compile-time OpenAPI documentation generation via derive macros.
//!   Enable with the `utoipa` feature.
//!
//! - **vespera**: OpenAPI 3.1 specification structures and route discovery.
//!   Enable with the `vespera` feature.
//!
//! # Route-Level Integration
//!
//! Both integrations support route-level OpenAPI metadata:
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
//! # Examples
//!
//! ## Using utoipa
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
//! ## Using vespera
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

/// OpenAPI metadata that can be attached to a route.
///
/// This struct stores operation-level OpenAPI information that can be
/// used to generate OpenAPI specifications from Tako routes.
#[derive(Clone, Debug, Default)]
pub struct RouteOpenApi {
    /// Unique identifier for the operation.
    pub operation_id: Option<String>,
    /// Short summary of the operation.
    pub summary: Option<String>,
    /// Detailed description of the operation.
    pub description: Option<String>,
    /// Tags for grouping operations.
    pub tags: Vec<String>,
    /// Whether the operation is deprecated.
    pub deprecated: bool,
    /// Response descriptions keyed by status code.
    pub responses: BTreeMap<u16, String>,
    /// Parameter descriptions.
    pub parameters: Vec<OpenApiParameter>,
    /// Request body description.
    pub request_body: Option<OpenApiRequestBody>,
    /// Security requirements.
    pub security: Vec<String>,
}

/// OpenAPI parameter definition.
#[derive(Clone, Debug)]
pub struct OpenApiParameter {
    /// Parameter name.
    pub name: String,
    /// Parameter location (query, header, path, cookie).
    pub location: ParameterLocation,
    /// Parameter description.
    pub description: Option<String>,
    /// Whether the parameter is required.
    pub required: bool,
}

/// Location of an OpenAPI parameter.
#[derive(Clone, Debug, Default)]
pub enum ParameterLocation {
    #[default]
    Query,
    Header,
    Path,
    Cookie,
}

/// OpenAPI request body definition.
#[derive(Clone, Debug)]
pub struct OpenApiRequestBody {
    /// Description of the request body.
    pub description: Option<String>,
    /// Whether the request body is required.
    pub required: bool,
    /// Content type (e.g., "application/json").
    pub content_type: String,
    /// Schema properties for the request body (name, type, description).
    pub schema_properties: Vec<RequestBodyProperty>,
}

/// A property definition for request body schema.
#[derive(Clone, Debug)]
pub struct RequestBodyProperty {
    /// Property name.
    pub name: String,
    /// Property type (e.g., "string", "integer", "boolean").
    pub property_type: String,
    /// Property description.
    pub description: Option<String>,
}

#[cfg(feature = "utoipa")]
#[cfg_attr(docsrs, doc(cfg(feature = "utoipa")))]
pub mod utoipa;

#[cfg(feature = "vespera")]
#[cfg_attr(docsrs, doc(cfg(feature = "vespera")))]
pub mod vespera;

/// OpenAPI UI helpers (Swagger UI, Scalar, RapiDoc, Redoc).
pub mod ui;
