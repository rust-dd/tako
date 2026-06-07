//! `OpenAPI` metadata attachment for a route.
//!
//! The chainable builder methods that record `OpenAPI` documentation
//! (operation id, summary, description, tags, deprecation, responses,
//! parameters, request body, security) onto the route's `RouteOpenApi`
//! store, plus the accessor that reads it back. Compiled only when an
//! `OpenAPI` backend feature is enabled.

#![cfg(any(feature = "utoipa", feature = "vespera"))]

use super::Route;
use crate::openapi::RouteOpenApi;

impl Route {
  /// Sets a unique operation ID for this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users", list_users)
  ///     .operation_id("listUsers");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn operation_id(&self, id: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.operation_id = Some(id.into());
    self
  }

  /// Sets a short summary for this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users/{id}", get_user)
  ///     .summary("Get user by ID");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn summary(&self, summary: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.summary = Some(summary.into());
    self
  }

  /// Sets a detailed description for this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users/{id}", get_user)
  ///     .description("Retrieves a user by their unique identifier");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn description(&self, description: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.description = Some(description.into());
    self
  }

  /// Adds a tag to group this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users", list_users)
  ///     .tag("users")
  ///     .tag("public");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn tag(&self, tag: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.tags.push(tag.into());
    self
  }

  /// Marks this route as deprecated in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/v1/users", list_users_v1)
  ///     .deprecated();
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn deprecated(&self) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.deprecated = true;
    self
  }

  /// Adds a response description for a status code in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::GET, "/users/{id}", get_user)
  ///     .response(200, "Successful response with user data")
  ///     .response(404, "User not found");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn response(&self, status: u16, description: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.responses.insert(status, description.into());
    self
  }

  /// Adds a parameter definition for this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::openapi::{OpenApiParameter, ParameterLocation};
  ///
  /// router.route(Method::GET, "/users", list_users)
  ///     .parameter(OpenApiParameter {
  ///         name: "limit".to_string(),
  ///         location: ParameterLocation::Query,
  ///         description: Some("Maximum number of results".to_string()),
  ///         required: false,
  ///     });
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn parameter(&self, param: crate::openapi::OpenApiParameter) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.parameters.push(param);
    self
  }

  /// Sets the request body description for this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// use tako::openapi::OpenApiRequestBody;
  ///
  /// router.route(Method::POST, "/users", create_user)
  ///     .request_body(OpenApiRequestBody {
  ///         description: Some("User data to create".to_string()),
  ///         required: true,
  ///         content_type: "application/json".to_string(),
  ///     });
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn request_body(&self, body: crate::openapi::OpenApiRequestBody) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.request_body = Some(body);
    self
  }

  /// Adds a security requirement for this route in `OpenAPI` documentation.
  ///
  /// # Examples
  ///
  /// ```rust,ignore
  /// router.route(Method::DELETE, "/users/{id}", delete_user)
  ///     .security("bearerAuth");
  /// ```
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn security(&self, requirement: impl Into<String>) -> &Self {
    let mut guard = self.openapi.write();
    let openapi = guard.get_or_insert_with(RouteOpenApi::default);
    openapi.security.push(requirement.into());
    self
  }

  /// Returns a clone of the `OpenAPI` metadata for this route, if any.
  #[cfg(any(feature = "utoipa", feature = "vespera"))]
  #[cfg_attr(docsrs, doc(cfg(any(feature = "utoipa", feature = "vespera"))))]
  pub fn openapi_metadata(&self) -> Option<RouteOpenApi> {
    self.openapi.read().clone()
  }
}
