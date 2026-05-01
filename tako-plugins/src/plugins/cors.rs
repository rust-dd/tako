#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
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

use std::fmt;
use std::sync::Arc;

use anyhow::Result;
use http::HeaderName;
use http::HeaderValue;
use http::Method;
use http::StatusCode;
use http::header::ACCESS_CONTROL_ALLOW_CREDENTIALS;
use http::header::ACCESS_CONTROL_ALLOW_HEADERS;
use http::header::ACCESS_CONTROL_ALLOW_METHODS;
use http::header::ACCESS_CONTROL_ALLOW_ORIGIN;
use http::header::ACCESS_CONTROL_MAX_AGE;
use http::header::ACCESS_CONTROL_REQUEST_HEADERS;
use http::header::ORIGIN;
use http::header::VARY;
use tako_core::body::TakoBody;
use tako_core::middleware::Next;
use tako_core::plugins::TakoPlugin;
use tako_core::responder::Responder;
use tako_core::router::Router;
use tako_core::types::Request;
use tako_core::types::Response;

/// Origin matching mode.
#[derive(Clone)]
pub enum OriginMatcher {
  /// Exact match (current default).
  Exact(String),
  /// Suffix match — `acme.example.com` matches origin `https://api.acme.example.com`.
  Suffix(String),
  /// Custom predicate. Receives the verbatim `Origin` header value.
  Custom(Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>),
}

impl OriginMatcher {
  fn matches(&self, origin: &str) -> bool {
    match self {
      Self::Exact(s) => s == origin,
      Self::Suffix(s) => {
        // Match against the host portion (`scheme://host[:port]`).
        let host = origin
          .splitn(4, '/')
          .nth(2)
          .unwrap_or(origin)
          .splitn(2, ':')
          .next()
          .unwrap_or("");
        host == s.as_str() || host.ends_with(&format!(".{s}"))
      }
      Self::Custom(f) => f(origin),
    }
  }
}

impl<S: Into<String>> From<S> for OriginMatcher {
  fn from(value: S) -> Self {
    Self::Exact(value.into())
  }
}

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
  /// Exact origin allow-list (legacy). For wider matching, use [`Self::origin_matchers`].
  pub origins: Vec<String>,
  /// Suffix / regex / custom origin matchers (additive on top of `origins`).
  pub origin_matchers: Vec<OriginMatcher>,
  /// List of allowed HTTP methods for cross-origin requests.
  pub methods: Vec<Method>,
  /// List of allowed request headers for cross-origin requests.
  pub headers: Vec<HeaderName>,
  /// Whether to allow credentials (cookies, authorization headers) in cross-origin requests.
  pub allow_credentials: bool,
  /// Maximum age in seconds for preflight request caching by browsers.
  pub max_age_secs: Option<u32>,
  /// Send `Access-Control-Allow-Private-Network: true` in preflight responses
  /// when the client signals `Access-Control-Request-Private-Network: true`.
  /// Required for browsers to allow public→private requests post Chrome 104.
  pub allow_private_network: bool,
}

impl Default for Config {
  /// Provides permissive default CORS configuration suitable for development.
  fn default() -> Self {
    Self {
      origins: Vec::new(),
      origin_matchers: Vec::new(),
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
      allow_private_network: false,
    }
  }
}

impl Config {
  /// Validates the CORS configuration against the Fetch spec's hard rules.
  ///
  /// Returns an error if the configuration would produce a header combination that
  /// browsers reject (e.g. `Access-Control-Allow-Origin: *` together with
  /// `Access-Control-Allow-Credentials: true`).
  pub fn validate(&self) -> Result<(), CorsConfigError> {
    if self.allow_credentials && self.origins.is_empty() && self.origin_matchers.is_empty() {
      return Err(CorsConfigError::CredentialsWithWildcardOrigin);
    }
    Ok(())
  }

  fn origin_allowed(&self, origin: &str) -> bool {
    self.origins.iter().any(|p| p == origin)
      || self.origin_matchers.iter().any(|m| m.matches(origin))
  }
}

/// Errors produced when constructing an invalid [`CorsPlugin`] configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorsConfigError {
  /// `allow_credentials = true` was combined with no explicit origins, which would
  /// produce `Access-Control-Allow-Origin: *` alongside `Access-Control-Allow-Credentials: true`.
  /// Browsers reject this combination per the Fetch spec.
  CredentialsWithWildcardOrigin,
}

impl fmt::Display for CorsConfigError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::CredentialsWithWildcardOrigin => f.write_str(
        "CORS misconfiguration: allow_credentials = true requires at least one explicit \
         allowed origin; reflecting `*` together with credentials is rejected by browsers",
      ),
    }
  }
}

impl std::error::Error for CorsConfigError {}

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
#[must_use]
pub struct CorsBuilder(Config);

impl Default for CorsBuilder {
  #[inline]
  fn default() -> Self {
    Self::new()
  }
}

impl CorsBuilder {
  /// Creates a new CORS configuration builder with default settings.
  #[inline]
  #[must_use]
  pub fn new() -> Self {
    Self(Config::default())
  }

  /// Adds an allowed origin to the CORS policy.
  #[inline]
  #[must_use]
  pub fn allow_origin(mut self, o: impl Into<String>) -> Self {
    self.0.origins.push(o.into());
    self
  }

  /// Sets the allowed HTTP methods for cross-origin requests.
  #[inline]
  #[must_use]
  pub fn allow_methods(mut self, m: &[Method]) -> Self {
    self.0.methods = m.to_vec();
    self
  }

  /// Sets the allowed request headers for cross-origin requests.
  #[inline]
  #[must_use]
  pub fn allow_headers(mut self, h: &[HeaderName]) -> Self {
    self.0.headers = h.to_vec();
    self
  }

  /// Enables or disables credential sharing in cross-origin requests.
  #[inline]
  #[must_use]
  pub fn allow_credentials(mut self, allow: bool) -> Self {
    self.0.allow_credentials = allow;
    self
  }

  /// Sets the maximum age for preflight request caching.
  #[inline]
  #[must_use]
  pub fn max_age_secs(mut self, secs: u32) -> Self {
    self.0.max_age_secs = Some(secs);
    self
  }

  /// Adds a suffix-style origin match (e.g. `example.com` accepts every
  /// subdomain). Combine with [`Self::allow_origin`] for hybrid policies.
  #[inline]
  #[must_use]
  pub fn allow_origin_suffix(mut self, suffix: impl Into<String>) -> Self {
    self.0.origin_matchers.push(OriginMatcher::Suffix(suffix.into()));
    self
  }

  /// Plug a custom origin predicate.
  #[inline]
  #[must_use]
  pub fn allow_origin_predicate<F>(mut self, f: F) -> Self
  where
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    self.0.origin_matchers.push(OriginMatcher::Custom(Arc::new(f)));
    self
  }

  /// Enables Private Network Access (Chrome PNA) preflight handling.
  #[inline]
  #[must_use]
  pub fn allow_private_network(mut self, yes: bool) -> Self {
    self.0.allow_private_network = yes;
    self
  }

  /// Builds the CORS plugin with the configured settings.
  ///
  /// # Panics
  ///
  /// Panics if [`Config::validate`] fails — typically when `allow_credentials = true`
  /// is combined with an empty origin list. Use [`CorsBuilder::try_build`] to handle
  /// the error explicitly.
  #[inline]
  #[must_use]
  pub fn build(self) -> CorsPlugin {
    self.try_build().expect("invalid CORS configuration")
  }

  /// Builds the CORS plugin, returning an error on invalid configuration instead of panicking.
  #[inline]
  pub fn try_build(self) -> Result<CorsPlugin, CorsConfigError> {
    self.0.validate()?;
    Ok(CorsPlugin { cfg: self.0 })
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
#[doc(alias = "cors")]
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
  let request_headers = req.headers().get(ACCESS_CONTROL_REQUEST_HEADERS).cloned();
  let pna_request = req
    .headers()
    .get("access-control-request-private-network")
    .and_then(|v| v.to_str().ok())
    .map(|v| v.eq_ignore_ascii_case("true"))
    .unwrap_or(false);

  if req.method() == Method::OPTIONS {
    let mut resp = http::Response::builder()
      .status(StatusCode::NO_CONTENT)
      .body(TakoBody::empty())
      .expect("valid CORS preflight response");
    add_cors_headers(
      &cfg,
      origin,
      request_headers.as_ref(),
      pna_request,
      &mut resp,
    );
    return resp.into_response();
  }

  let mut resp = next.run(req).await;
  add_cors_headers(&cfg, origin, request_headers.as_ref(), false, &mut resp);
  resp.into_response()
}

/// Adds CORS headers to HTTP responses based on configuration and request origin.
fn add_cors_headers(
  cfg: &Config,
  origin: Option<HeaderValue>,
  request_headers: Option<&HeaderValue>,
  pna_request: bool,
  resp: &mut Response,
) {
  // Origin validation and Access-Control-Allow-Origin header.
  //
  // Invariant guarded by `Config::validate`: when `allow_credentials = true`,
  // at least one origin or matcher is configured — so `*` is never emitted
  // alongside credentials.
  let allow_anything = cfg.origins.is_empty() && cfg.origin_matchers.is_empty();
  let (allow_origin, mirrored_origin) = if allow_anything {
    ("*".to_string(), false)
  } else if let Some(o) = &origin {
    let s = o.to_str().unwrap_or_default();
    if cfg.origin_allowed(s) {
      (s.to_string(), true)
    } else {
      return;
    }
  } else {
    return;
  };

  resp.headers_mut().insert(
    ACCESS_CONTROL_ALLOW_ORIGIN,
    HeaderValue::from_str(&allow_origin).expect("valid origin header value"),
  );

  // When the response varies on the request Origin (i.e. we mirrored it back),
  // shared caches must key on Origin to avoid cross-origin response leakage.
  if mirrored_origin {
    resp
      .headers_mut()
      .append(VARY, HeaderValue::from_static("Origin"));
  }

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
      HeaderValue::from_str(&v).expect("valid methods header value"),
    );
  }

  // Access-Control-Allow-Headers header.
  //
  // `*` is invalid in any "Allow-*" header when `Access-Control-Allow-Credentials: true`
  // (Fetch spec). Two strategies when no explicit list is configured:
  //   - credentials disallowed: emit `*` (browsers accept it).
  //   - credentials allowed: reflect the request's `Access-Control-Request-Headers`
  //     so the preflight succeeds without a footgun.
  if cfg.headers.is_empty() {
    if cfg.allow_credentials {
      if let Some(req_h) = request_headers {
        resp
          .headers_mut()
          .insert(ACCESS_CONTROL_ALLOW_HEADERS, req_h.clone());
        resp.headers_mut().append(
          VARY,
          HeaderValue::from_static("Access-Control-Request-Headers"),
        );
      }
      // No `Access-Control-Request-Headers` to reflect → emit nothing.
    } else {
      resp
        .headers_mut()
        .insert(ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("*"));
    }
  } else {
    let h = cfg
      .headers
      .iter()
      .map(|h| h.as_str())
      .collect::<Vec<_>>()
      .join(",");
    resp.headers_mut().insert(
      ACCESS_CONTROL_ALLOW_HEADERS,
      HeaderValue::from_str(&h).expect("valid headers header value"),
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
      HeaderValue::from_str(&secs.to_string()).expect("valid max-age header value"),
    );
  }

  // Private Network Access (PNA) — emit only on preflight responses where
  // the client signaled the request bit. Doing so on regular responses is a
  // spec violation.
  if cfg.allow_private_network && pna_request {
    resp.headers_mut().insert(
      "access-control-allow-private-network",
      HeaderValue::from_static("true"),
    );
  }
}
