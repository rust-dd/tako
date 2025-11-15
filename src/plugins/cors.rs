//! Cross-Origin Resource Sharing (CORS) plugin for handling cross-origin HTTP requests.
//!
//! This module provides comprehensive CORS support for Tako web applications, enabling
//! secure cross-origin resource sharing between different domains. The plugin handles
//! preflight OPTIONS requests, validates origins against configured policies, and adds
//! appropriate CORS headers to responses. It supports configurable origins, methods,
//! headers, credentials, and cache control for flexible cross-origin access policies.
//!
//! The CORS plugin can be applied at both router-level (all routes) and route-level
//! (specific routes), allowing fine-grained control over CORS policies.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::cors::{CorsPlugin, CorsBuilder};
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//! use http::Method;
//!
//! async fn api_handler(_req: tako::types::Request) -> &'static str {
//!     "API response"
//! }
//!
//! async fn public_handler(_req: tako::types::Request) -> &'static str {
//!     "Public response"
//! }
//!
//! let mut router = Router::new();
//!
//! // Router-level: Basic CORS setup allowing all origins (applied to all routes)
//! let global_cors = CorsBuilder::new().build();
//! router.plugin(global_cors);
//!
//! // Route-level: Restrictive CORS for specific API endpoint
//! let api_route = router.route(Method::GET, "/api/data", api_handler);
//! let api_cors = CorsBuilder::new()
//!     .allow_origin("https://app.example.com")
//!     .allow_origin("https://admin.example.com")
//!     .allow_methods(&[Method::GET, Method::POST, Method::PUT])
//!     .allow_credentials(true)
//!     .max_age_secs(86400)
//!     .build();
//! api_route.plugin(api_cors);
//!
//! // Another route without CORS restrictions (uses global if set)
//! router.route(Method::GET, "/public", public_handler);
//! ```

use anyhow::Result;
use http::{
  HeaderName, HeaderValue, Method, StatusCode,
  header::{
    ACCESS_CONTROL_ALLOW_CREDENTIALS, ACCESS_CONTROL_ALLOW_HEADERS, ACCESS_CONTROL_ALLOW_METHODS,
    ACCESS_CONTROL_ALLOW_ORIGIN, ACCESS_CONTROL_MAX_AGE, ORIGIN,
  },
};

use crate::{
  body::TakoBody,
  middleware::Next,
  plugins::TakoPlugin,
  responder::Responder,
  router::Router,
  types::{Request, Response},
};

/// CORS policy configuration settings for cross-origin request handling.
///
/// `Config` defines the Cross-Origin Resource Sharing policy including allowed origins,
/// HTTP methods, headers, credential handling, and preflight cache duration. The
/// configuration determines which cross-origin requests are permitted and what headers
/// are added to responses to enable secure cross-origin communication.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::cors::Config;
/// use http::{Method, HeaderName};
///
/// let config = Config {
///     origins: vec!["https://app.example.com".to_string()],
///     methods: vec![Method::GET, Method::POST],
///     headers: vec![HeaderName::from_static("x-api-key")],
///     allow_credentials: true,
///     max_age_secs: Some(3600),
/// };
/// ```
#[derive(Clone)]
pub struct Config {
  /// List of allowed origin URLs for cross-origin requests.
  pub origins: Vec<String>,
  /// List of allowed HTTP methods for cross-origin requests.
  pub methods: Vec<Method>,
  /// List of allowed request headers for cross-origin requests.
  pub headers: Vec<HeaderName>,
  /// Whether to allow credentials (cookies, authorization headers) in cross-origin requests.
  pub allow_credentials: bool,
  /// Maximum age in seconds for preflight request caching by browsers.
  pub max_age_secs: Option<u32>,
}

impl Default for Config {
  /// Provides permissive default CORS configuration suitable for development.
  fn default() -> Self {
    Self {
      origins: Vec::new(),
      methods: vec![
        Method::GET,
        Method::POST,
        Method::PUT,
        Method::PATCH,
        Method::DELETE,
        Method::OPTIONS,
      ],
      headers: Vec::new(),
      allow_credentials: false,
      max_age_secs: Some(3600),
    }
  }
}

/// Builder for configuring CORS policies with a fluent API.
///
/// `CorsBuilder` provides a convenient way to construct CORS configurations using
/// method chaining. It starts with sensible defaults and allows selective customization
/// of origins, methods, headers, and other CORS policy aspects. The builder pattern
/// ensures all configuration is explicit while maintaining ease of use.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::cors::CorsBuilder;
/// use http::{Method, HeaderName};
///
/// // Development setup - permissive CORS
/// let dev_cors = CorsBuilder::new()
///     .allow_credentials(false)
///     .build();
///
/// // Production setup - restrictive CORS
/// let prod_cors = CorsBuilder::new()
///     .allow_origin("https://app.mysite.com")
///     .allow_origin("https://admin.mysite.com")
///     .allow_methods(&[Method::GET, Method::POST])
///     .allow_headers(&[HeaderName::from_static("authorization")])
///     .allow_credentials(true)
///     .max_age_secs(86400)
///     .build();
/// ```
pub struct CorsBuilder(Config);

impl CorsBuilder {
  /// Creates a new CORS configuration builder with default settings.
  pub fn new() -> Self {
    Self(Config::default())
  }

  /// Adds an allowed origin to the CORS policy.
  pub fn allow_origin(mut self, o: impl Into<String>) -> Self {
    self.0.origins.push(o.into());
    self
  }

  /// Sets the allowed HTTP methods for cross-origin requests.
  pub fn allow_methods(mut self, m: &[Method]) -> Self {
    self.0.methods = m.to_vec();
    self
  }

  /// Sets the allowed request headers for cross-origin requests.
  pub fn allow_headers(mut self, h: &[HeaderName]) -> Self {
    self.0.headers = h.to_vec();
    self
  }

  /// Enables or disables credential sharing in cross-origin requests.
  pub fn allow_credentials(mut self, allow: bool) -> Self {
    self.0.allow_credentials = allow;
    self
  }

  /// Sets the maximum age for preflight request caching.
  pub fn max_age_secs(mut self, secs: u32) -> Self {
    self.0.max_age_secs = Some(secs);
    self
  }

  /// Builds the CORS plugin with the configured settings.
  pub fn build(self) -> CorsPlugin {
    CorsPlugin { cfg: self.0 }
  }
}

/// CORS plugin for handling cross-origin resource sharing in Tako applications.
///
/// `CorsPlugin` implements the TakoPlugin trait to provide comprehensive CORS support
/// including preflight request handling, origin validation, and response header
/// management. It automatically handles OPTIONS preflight requests and adds appropriate
/// CORS headers to all responses based on the configured policy.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::cors::{CorsPlugin, CorsBuilder};
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
/// use http::Method;
///
/// // Basic setup with default permissive policy
/// let cors = CorsPlugin::default();
/// let mut router = Router::new();
/// router.plugin(cors);
///
/// // Custom restrictive policy for production
/// let prod_cors = CorsBuilder::new()
///     .allow_origin("https://myapp.com")
///     .allow_methods(&[Method::GET, Method::POST])
///     .allow_credentials(true)
///     .build();
/// router.plugin(prod_cors);
/// ```
#[derive(Clone)]
pub struct CorsPlugin {
  cfg: Config,
}

impl Default for CorsPlugin {
  /// Creates a CORS plugin with permissive default configuration.
  fn default() -> Self {
    Self {
      cfg: Config::default(),
    }
  }
}

impl TakoPlugin for CorsPlugin {
  /// Returns the plugin name for identification and debugging.
  fn name(&self) -> &'static str {
    "CorsPlugin"
  }

  /// Sets up the CORS plugin by registering middleware with the router.
  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = self.cfg.clone();
    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      async move { handle_cors(req, next, cfg).await }
    });
    Ok(())
  }
}

/// Handles CORS processing for incoming requests including preflight and actual requests.
async fn handle_cors(req: Request, next: Next, cfg: Config) -> impl Responder {
  let origin = req.headers().get(ORIGIN).cloned();

  if req.method() == Method::OPTIONS {
    let mut resp = http::Response::builder()
      .status(StatusCode::NO_CONTENT)
      .body(TakoBody::empty())
      .unwrap();
    add_cors_headers(&cfg, origin, &mut resp);
    return resp.into_response();
  }

  let mut resp = next.run(req).await;
  add_cors_headers(&cfg, origin, &mut resp);
  resp.into_response()
}

/// Adds CORS headers to HTTP responses based on configuration and request origin.
fn add_cors_headers(cfg: &Config, origin: Option<HeaderValue>, resp: &mut Response) {
  // Origin validation and Access-Control-Allow-Origin header
  let allow_origin = if cfg.origins.is_empty() {
    "*".to_string()
  } else if let Some(o) = &origin {
    let s = o.to_str().unwrap_or_default();
    if cfg.origins.iter().any(|p| p == s) {
      s.to_string()
    } else {
      return; // Origin not allowed, don't add CORS headers
    }
  } else {
    return; // No origin header, don't add CORS headers
  };

  resp.headers_mut().insert(
    ACCESS_CONTROL_ALLOW_ORIGIN,
    HeaderValue::from_str(&allow_origin).unwrap(),
  );

  // Access-Control-Allow-Methods header
  let methods = if cfg.methods.is_empty() {
    None
  } else {
    Some(
      cfg
        .methods
        .iter()
        .map(|m| m.as_str())
        .collect::<Vec<_>>()
        .join(","),
    )
  };
  if let Some(v) = methods {
    resp.headers_mut().insert(
      ACCESS_CONTROL_ALLOW_METHODS,
      HeaderValue::from_str(&v).unwrap(),
    );
  }

  // Access-Control-Allow-Headers header
  if !cfg.headers.is_empty() {
    let h = cfg
      .headers
      .iter()
      .map(|h| h.as_str())
      .collect::<Vec<_>>()
      .join(",");
    resp.headers_mut().insert(
      ACCESS_CONTROL_ALLOW_HEADERS,
      HeaderValue::from_str(&h).unwrap(),
    );
  }

  // Access-Control-Allow-Credentials header
  if cfg.allow_credentials {
    resp.headers_mut().insert(
      ACCESS_CONTROL_ALLOW_CREDENTIALS,
      HeaderValue::from_static("true"),
    );
  }

  // Access-Control-Max-Age header
  if let Some(secs) = cfg.max_age_secs {
    resp.headers_mut().insert(
      ACCESS_CONTROL_MAX_AGE,
      HeaderValue::from_str(&secs.to_string()).unwrap(),
    );
  }
}
