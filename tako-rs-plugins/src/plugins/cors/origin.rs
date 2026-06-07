use std::sync::Arc;

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
  pub(crate) fn matches(&self, origin: &str) -> bool {
    match self {
      Self::Exact(s) => s == origin,
      Self::Suffix(s) => {
        // PPL-20: parse the host with `url::Url` instead of the prior
        // `split('/').nth(2).split(':')` chain, which mishandled
        // trailing slashes (Origin headers should not have them, but
        // browsers occasionally do), IPv6 literals like
        // `https://[::1]:8443` (the `split(':')` would chop the literal
        // mid-address), and userinfo like `https://user@example.com`
        // (the host would have leaked the userinfo prefix).
        let host = url::Url::parse(origin)
          .ok()
          .and_then(|u| u.host_str().map(str::to_owned))
          .unwrap_or_default();
        if host.is_empty() {
          return false;
        }
        host == *s.as_str() || host.ends_with(&format!(".{s}"))
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
