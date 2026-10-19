use std::sync::Arc;

#[derive(Clone)]
pub enum OriginMatcher {
  /// What a plain string converts into.
  Exact(String),
  /// Suffix match — `acme.example.com` matches origin `https://api.acme.example.com`.
  Suffix(String),
  /// Custom predicate. Receives the verbatim `Origin` header value.
  Custom(Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>),
}

impl OriginMatcher {
  pub(crate) fn matches(&self, origin: &str) -> bool {
    match self {
      Self::Exact(s) => s == origin,
      Self::Suffix(s) => {
        // `Url` handles IPv6 literals and userinfo that naive splitting gets wrong.
        let Ok(url) = url::Url::parse(origin) else {
          return false;
        };
        let host = url.host_str().unwrap_or_default();
        !host.is_empty()
          && host
            .strip_suffix(s.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.ends_with('.'))
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
