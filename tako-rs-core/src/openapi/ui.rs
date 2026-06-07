//! `OpenAPI` UI helpers for serving interactive API documentation.
//!
//! This module provides responders for serving popular `OpenAPI` UI interfaces:
//! - **Swagger UI**: The classic `OpenAPI` documentation interface
//! - **Scalar**: A modern, beautiful API documentation UI
//! - **`RapiDoc`**: A feature-rich API documentation viewer
//!
//! All UIs are served via CDN, requiring no additional dependencies.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::openapi::ui::{SwaggerUi, Scalar, RapiDoc};
//! use tako::{router::Router, Method};
//!
//! let mut router = Router::new();
//!
//! // Serve Swagger UI at /docs
//! router.route(Method::GET, "/docs", |_| async {
//!     SwaggerUi::new("/openapi.json")
//! });
//!
//! // Serve Scalar at /scalar
//! router.route(Method::GET, "/scalar", |_| async {
//!     Scalar::new("/openapi.json")
//! });
//!
//! // Serve RapiDoc at /rapidoc
//! router.route(Method::GET, "/rapidoc", |_| async {
//!     RapiDoc::new("/openapi.json")
//! });
//! ```

mod escape;
mod rapidoc;
mod redoc;
mod scalar;
mod swagger;

pub use rapidoc::RapiDoc;
pub use rapidoc::RapiDocRenderStyle;
pub use rapidoc::RapiDocTheme;
pub use redoc::Redoc;
pub use scalar::Scalar;
pub use scalar::ScalarTheme;
pub use swagger::SwaggerUi;
