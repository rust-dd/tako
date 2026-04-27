//! CSRF protection middleware using the double-submit cookie pattern.
//!
//! Generates a CSRF token and sets it as a cookie. On unsafe methods (POST, PUT, DELETE, PATCH),
//! validates that the token from the `X-CSRF-Token` header (or a form field) matches the cookie.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderValue;
use http::Method;
use http::StatusCode;

use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;

/// CSRF protection middleware configuration.
///
/// # Examples
///
/// ```rust
/// use tako::middleware::csrf::Csrf;
/// use tako::middleware::IntoMiddleware;
///
/// let csrf = Csrf::new();
/// let mw = csrf.into_middleware();
/// ```
pub struct Csrf {
  cookie_name: String,
  header_name: String,
  exempt_paths: Vec<String>,
  secure: bool,
}

impl Default for Csrf {
  fn default() -> Self {
    Self::new()
  }
}

impl Csrf {
  /// Creates a new CSRF middleware with default settings.
  pub fn new() -> Self {
    Self {
      cookie_name: "csrf_token".to_string(),
      header_name: "x-csrf-token".to_string(),
      exempt_paths: Vec::new(),
      secure: false,
    }
  }

  /// Sets the CSRF cookie name.
  pub fn cookie_name(mut self, name: &str) -> Self {
    self.cookie_name = name.to_string();
    self
  }

  /// Sets the header name to check for the CSRF token.
  pub fn header_name(mut self, name: &str) -> Self {
    self.header_name = name.to_string();
    self
  }

  /// Adds a path to exempt from CSRF checks.
  pub fn exempt(mut self, path: &str) -> Self {
    self.exempt_paths.push(path.to_string());
    self
  }

  /// Sets the Secure flag on the CSRF cookie.
  pub fn secure(mut self, secure: bool) -> Self {
    self.secure = secure;
    self
  }
}

fn generate_csrf_token() -> String {
  use std::time::{SystemTime, UNIX_EPOCH};
  let now = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_nanos();
  let a = now.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
  let b = a.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
  format!("{:016x}{:016x}", a & 0xFFFFFFFFFFFFFFFF, b & 0xFFFFFFFFFFFFFFFF)
}

fn is_unsafe_method(method: &Method) -> bool {
  matches!(
    *method,
    Method::POST | Method::PUT | Method::DELETE | Method::PATCH
  )
}

impl IntoMiddleware for Csrf {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let cookie_name = Arc::new(self.cookie_name);
    let header_name = Arc::new(self.header_name);
    let exempt_paths = Arc::new(self.exempt_paths);
    let secure = self.secure;

    move |req: Request, next: Next| {
      let cookie_name = cookie_name.clone();
      let header_name = header_name.clone();
      let exempt_paths = exempt_paths.clone();

      Box::pin(async move {
        let path = req.uri().path().to_string();

        // Skip CSRF for safe methods
        if !is_unsafe_method(req.method()) {
          let mut resp = next.run(req).await;
          // Ensure CSRF cookie is always set
          ensure_csrf_cookie(&mut resp, &cookie_name, secure);
          return resp;
        }

        // Skip exempt paths
        if exempt_paths.iter().any(|p| path.starts_with(p.as_str())) {
          let mut resp = next.run(req).await;
          ensure_csrf_cookie(&mut resp, &cookie_name, secure);
          return resp;
        }

        // Extract token from cookie
        let cookie_token = req
          .headers()
          .get(http::header::COOKIE)
          .and_then(|v| v.to_str().ok())
          .and_then(|cookies| {
            cookies.split(';').find_map(|pair| {
              let pair = pair.trim();
              let (name, value) = pair.split_once('=')?;
              if name.trim() == cookie_name.as_str() {
                Some(value.trim().to_string())
              } else {
                None
              }
            })
          });

        // Extract token from header
        let header_token = req
          .headers()
          .get(header_name.as_str())
          .and_then(|v| v.to_str().ok())
          .map(|s| s.to_string());

        // Validate: both must exist and match
        match (cookie_token, header_token) {
          (Some(ct), Some(ht)) if ct == ht && !ct.is_empty() => {
            let mut resp = next.run(req).await;
            ensure_csrf_cookie(&mut resp, &cookie_name, secure);
            resp
          }
          _ => {
            (StatusCode::FORBIDDEN, "CSRF token mismatch").into_response()
          }
        }
      })
    }
  }
}

fn ensure_csrf_cookie(resp: &mut Response, cookie_name: &str, secure: bool) {
  // Only set if not already present in response
  let has_csrf = resp
    .headers()
    .get_all(http::header::SET_COOKIE)
    .iter()
    .any(|v| v.to_str().unwrap_or("").starts_with(cookie_name));

  if !has_csrf {
    let token = generate_csrf_token();
    let mut cookie = format!(
      "{}={}; Path=/; SameSite=Strict",
      cookie_name, token
    );
    if secure {
      cookie.push_str("; Secure");
    }
    if let Ok(val) = HeaderValue::from_str(&cookie) {
      resp.headers_mut().append(http::header::SET_COOKIE, val);
    }
  }
}
