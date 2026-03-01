//! Security headers middleware for common HTTP security best practices.
//!
//! Adds standard security headers to all responses:
//! - `X-Content-Type-Options: nosniff`
//! - `X-Frame-Options: DENY`
//! - `X-XSS-Protection: 0`
//! - `Referrer-Policy: strict-origin-when-cross-origin`
//! - `Strict-Transport-Security` (opt-in via builder)

use std::future::Future;
use std::pin::Pin;

use http::HeaderValue;

use crate::middleware::IntoMiddleware;
use crate::middleware::Next;
use crate::types::Request;
use crate::types::Response;

/// Security headers middleware configuration.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::security_headers::SecurityHeaders;
/// use tako::middleware::IntoMiddleware;
///
/// let headers = SecurityHeaders::new();
/// let mw = headers.into_middleware();
///
/// // With HSTS enabled
/// let headers = SecurityHeaders::new().hsts(true);
/// ```
pub struct SecurityHeaders {
  frame_options: HeaderValue,
  hsts: bool,
  hsts_max_age: u64,
  hsts_include_subdomains: bool,
  referrer_policy: HeaderValue,
}

impl Default for SecurityHeaders {
  fn default() -> Self {
    Self::new()
  }
}

impl SecurityHeaders {
  /// Creates a new SecurityHeaders middleware with sensible defaults.
  pub fn new() -> Self {
    Self {
      frame_options: HeaderValue::from_static("DENY"),
      hsts: false,
      hsts_max_age: 31536000,
      hsts_include_subdomains: true,
      referrer_policy: HeaderValue::from_static("strict-origin-when-cross-origin"),
    }
  }

  /// Sets the `X-Frame-Options` value (e.g., "DENY", "SAMEORIGIN").
  pub fn frame_options(mut self, value: &'static str) -> Self {
    self.frame_options = HeaderValue::from_static(value);
    self
  }

  /// Enables or disables `Strict-Transport-Security` header.
  pub fn hsts(mut self, enable: bool) -> Self {
    self.hsts = enable;
    self
  }

  /// Sets the HSTS `max-age` value in seconds. Default: 31536000 (1 year).
  pub fn hsts_max_age(mut self, seconds: u64) -> Self {
    self.hsts_max_age = seconds;
    self
  }

  /// Sets the `Referrer-Policy` value.
  pub fn referrer_policy(mut self, value: &'static str) -> Self {
    self.referrer_policy = HeaderValue::from_static(value);
    self
  }
}

impl IntoMiddleware for SecurityHeaders {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let frame_options = self.frame_options;
    let hsts_value = if self.hsts {
      let val = if self.hsts_include_subdomains {
        format!(
          "max-age={}; includeSubDomains",
          self.hsts_max_age
        )
      } else {
        format!("max-age={}", self.hsts_max_age)
      };
      Some(HeaderValue::from_str(&val).expect("valid HSTS header"))
    } else {
      None
    };
    let referrer_policy = self.referrer_policy;

    move |req: Request, next: Next| {
      let frame_options = frame_options.clone();
      let hsts_value = hsts_value.clone();
      let referrer_policy = referrer_policy.clone();

      Box::pin(async move {
        let mut resp = next.run(req).await;
        let headers = resp.headers_mut();

        headers.insert("x-content-type-options", HeaderValue::from_static("nosniff"));
        headers.insert("x-frame-options", frame_options);
        headers.insert("x-xss-protection", HeaderValue::from_static("0"));
        headers.insert("referrer-policy", referrer_policy);

        if let Some(hsts) = hsts_value {
          headers.insert("strict-transport-security", hsts);
        }

        resp
      })
    }
  }
}
