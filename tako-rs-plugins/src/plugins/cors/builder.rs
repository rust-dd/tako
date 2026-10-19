use std::sync::Arc;

use http::HeaderName;
use http::Method;

use super::config::Config;
use super::config::CorsConfigError;
use super::origin::OriginMatcher;
use super::plugin::CorsPlugin;

/// With no origin configured, every origin is allowed and `*` is sent.
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
  #[inline]
  pub fn new() -> Self {
    Self(Config::default())
  }

  /// Exact match against the `Origin` header value.
  #[inline]
  pub fn allow_origin(mut self, o: impl Into<String>) -> Self {
    self.0.origins.push(o.into());
    self
  }

  /// Replaces the default GET, POST, PUT, PATCH, DELETE and OPTIONS.
  #[inline]
  pub fn allow_methods(mut self, m: &[Method]) -> Self {
    self.0.methods = m.to_vec();
    self
  }

  /// Without a list, `*` is sent, or with credentials the requested headers are reflected.
  #[inline]
  pub fn allow_headers(mut self, h: &[HeaderName]) -> Self {
    self.0.headers = h.to_vec();
    self
  }

  /// Requires at least one allowed origin or matcher.
  #[inline]
  pub fn allow_credentials(mut self, allow: bool) -> Self {
    self.0.allow_credentials = allow;
    self
  }

  /// Defaults to one hour.
  #[inline]
  pub fn max_age_secs(mut self, secs: u32) -> Self {
    self.0.max_age_secs = Some(secs);
    self
  }

  /// Adds a suffix-style origin match (e.g. `example.com` accepts every
  /// subdomain). Combine with [`Self::allow_origin`] for hybrid policies.
  #[inline]
  pub fn allow_origin_suffix(mut self, suffix: impl Into<String>) -> Self {
    self
      .0
      .origin_matchers
      .push(OriginMatcher::Suffix(suffix.into()));
    self
  }

  /// Receives the raw `Origin` header value.
  #[inline]
  pub fn allow_origin_predicate<F>(mut self, f: F) -> Self
  where
    F: Fn(&str) -> bool + Send + Sync + 'static,
  {
    self
      .0
      .origin_matchers
      .push(OriginMatcher::Custom(Arc::new(f)));
    self
  }

  /// Enables Private Network Access (Chrome PNA) preflight handling.
  #[inline]
  pub fn allow_private_network(mut self, yes: bool) -> Self {
    self.0.allow_private_network = yes;
    self
  }

  /// Panics when [`Config::validate`] fails; [`Self::try_build`] returns the error instead.
  #[inline]
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
