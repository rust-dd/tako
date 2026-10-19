use std::fmt;

use anyhow::Result;
use http::HeaderName;
use http::Method;

use super::origin::OriginMatcher;

/// With no origins or matchers, every origin is allowed and `*` is sent.
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
///     ..Default::default()
/// };
/// ```
#[derive(Clone)]
pub struct Config {
  /// Exact matches against the `Origin` header value.
  pub origins: Vec<String>,
  /// Checked in addition to `origins`.
  pub origin_matchers: Vec<OriginMatcher>,
  pub methods: Vec<Method>,
  /// Empty sends `*`, or with credentials reflects the requested headers.
  pub headers: Vec<HeaderName>,
  /// Requires an explicit origin; see [`Self::validate`].
  pub allow_credentials: bool,
  /// Preflight cache lifetime.
  pub max_age_secs: Option<u32>,
  /// Answers a preflight's `Access-Control-Request-Private-Network: true`, which Chrome
  /// requires before public pages may call private-network hosts.
  pub allow_private_network: bool,
}

impl Default for Config {
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
  /// Rejects header combinations browsers refuse, such as credentials with a `*` origin.
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
  /// Credentials without an explicit origin would pair them with `*`, which browsers reject.
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
