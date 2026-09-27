//! CSRF middleware configuration and builder methods.

use crate::middleware::session::SameSite;

/// CSRF middleware configuration.
pub struct Csrf {
  pub(crate) cookie_name: String,
  pub(crate) header_name: String,
  pub(crate) exempt_paths: Vec<String>,
  pub(crate) secure: bool,
  pub(crate) same_site: SameSite,
  pub(crate) trusted_origins: Vec<String>,
  pub(crate) bind_to_session: bool,
  pub(crate) session_key: String,
  pub(crate) store: Option<std::sync::Arc<dyn crate::stores::CsrfTokenStore>>,
  pub(crate) single_use: bool,
  pub(crate) token_ttl: std::time::Duration,
}

impl Default for Csrf {
  fn default() -> Self {
    Self::new()
  }
}

impl Csrf {
  /// Creates a CSRF middleware with the secure defaults.
  pub fn new() -> Self {
    Self {
      cookie_name: "csrf_token".to_string(),
      header_name: "x-csrf-token".to_string(),
      exempt_paths: Vec::new(),
      secure: true,
      same_site: SameSite::Strict,
      trusted_origins: Vec::new(),
      bind_to_session: true,
      session_key: "__csrf".to_string(),
      store: None,
      single_use: false,
      token_ttl: std::time::Duration::from_secs(3600),
    }
  }

  /// Use server-side token storage. Session middleware must run before CSRF.
  /// The default without a store is the session-bound double-submit cookie scheme.
  pub fn store(mut self, store: impl crate::stores::CsrfTokenStore) -> Self {
    self.store = Some(std::sync::Arc::new(store));
    self
  }

  /// Consume stored tokens on successful unsafe requests; defaults to false.
  pub fn single_use(mut self, enabled: bool) -> Self {
    self.single_use = enabled;
    self
  }

  /// Stored token lifetime, default one hour.
  pub fn token_ttl(mut self, ttl: std::time::Duration) -> Self {
    self.token_ttl = ttl;
    self
  }

  /// CSRF cookie name. Default: `"csrf_token"`.
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  /// Header name expected to carry the token. Default: `"x-csrf-token"`.
  pub fn header_name(mut self, name: &str) -> Self {
    self.header_name = name.to_string();
    self
  }

  /// Adds a path prefix that should bypass CSRF entirely (e.g. webhooks).
  pub fn exempt(mut self, path: &str) -> Self {
    self.exempt_paths.push(path.to_string());
    self
  }

  /// Toggle the cookie `Secure` flag (default true). Required when `same_site = None`.
  pub fn secure(mut self, secure: bool) -> Self {
    self.secure = secure;
    self
  }

  /// Override the `SameSite` attribute on the CSRF cookie.
  pub fn same_site(mut self, ss: SameSite) -> Self {
    self.same_site = ss;
    self
  }

  /// Origins to accept as fallback when cookie/header verification fails.
  /// Both `Origin` and `Referer` are matched (scheme + host\[:port\]).
  pub fn trust_origin(mut self, origin: impl Into<String>) -> Self {
    self.trusted_origins.push(origin.into());
    self
  }

  /// When true (default), the token is stored in the session under
  /// [`Self::session_key`] and bound to the active session id.
  pub fn bind_to_session(mut self, bind: bool) -> Self {
    self.bind_to_session = bind;
    self
  }

  /// Session key used to persist the token.
  pub fn session_key(mut self, k: &str) -> Self {
    self.session_key = k.to_string();
    self
  }
}
