use std::fmt;

use anyhow::Result;
use http::HeaderName;
use http::Method;

use super::origin::OriginMatcher;

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

  pub(crate) fn origin_allowed(&self, origin: &str) -> bool {
    self.origins.iter().any(|p| p == origin)
      || self.origin_matchers.iter().any(|m| m.matches(origin))
  }
}

/// Errors produced when constructing an invalid [`CorsPlugin`](super::CorsPlugin) configuration.
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
