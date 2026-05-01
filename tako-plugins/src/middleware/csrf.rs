//! CSRF protection middleware.
//!
//! Default mode is the **double-submit cookie** pattern: a random token is
//! placed in a cookie *and* must be echoed back in a request header (or form
//! field). The middleware verifies the two values match and that, when a
//! [`Session`](super::session::Session) extension is present, the cookie was
//! issued for the current session.
//!
//! v2 additions:
//!
//! - **Session-bound tokens.** When a [`Session`] extension is in scope, the
//!   token is stored in the session and the cookie value must agree with it.
//!   Tokens carried over from a previous session id (after privilege rotation)
//!   are rejected.
//! - **Origin / Referer fallback.** When neither cookie nor header is set
//!   (legacy clients) the middleware can fall back to a strict
//!   `Origin` / `Referer` allow-list before rejecting.
//! - **Configurable `SameSite`.** Defaults stay `Strict`. Choose `Lax` if
//!   the application embeds the API in a same-site form post flow.

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

use super::session::SameSite;
use super::session::Session;

/// CSRF middleware configuration.
pub struct Csrf {
  cookie_name: String,
  header_name: String,
  exempt_paths: Vec<String>,
  secure: bool,
  same_site: SameSite,
  trusted_origins: Vec<String>,
  bind_to_session: bool,
  session_key: String,
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
      secure: false,
      same_site: SameSite::Strict,
      trusted_origins: Vec::new(),
      bind_to_session: true,
      session_key: "__csrf".to_string(),
    }
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

  /// Toggle the cookie `Secure` flag. Required when `same_site = None`.
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
  /// Both `Origin` and `Referer` are matched (scheme + host[:port]).
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

fn generate_csrf_token() -> String {
  uuid::Uuid::new_v4().simple().to_string()
}

fn is_unsafe_method(method: &Method) -> bool {
  matches!(
    *method,
    Method::POST | Method::PUT | Method::DELETE | Method::PATCH
  )
}

fn extract_cookie<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
  req
    .headers()
    .get(http::header::COOKIE)
    .and_then(|v| v.to_str().ok())
    .and_then(|cookies| {
      cookies.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (k, v) = pair.split_once('=')?;
        if k.trim() == name {
          Some(v.trim())
        } else {
          None
        }
      })
    })
}

fn origin_allowed(value: &str, allow: &[String]) -> bool {
  // Match by exact scheme://host[:port] prefix, ignoring path.
  let target = value
    .splitn(4, '/')
    .take(3)
    .collect::<Vec<_>>()
    .join("/");
  allow.iter().any(|o| o == &target)
}

fn build_cookie(name: &str, token: &str, secure: bool, same_site: SameSite) -> String {
  let mut s = format!("{}={}; Path=/; SameSite={}", name, token, same_site_str(same_site));
  if secure || matches!(same_site, SameSite::None) {
    s.push_str("; Secure");
  }
  s
}

fn same_site_str(ss: SameSite) -> &'static str {
  match ss {
    SameSite::Strict => "Strict",
    SameSite::Lax => "Lax",
    SameSite::None => "None",
  }
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
    let same_site = self.same_site;
    let trusted_origins = Arc::new(self.trusted_origins);
    let bind_to_session = self.bind_to_session;
    let session_key = Arc::new(self.session_key);

    move |req: Request, next: Next| {
      let cookie_name = cookie_name.clone();
      let header_name = header_name.clone();
      let exempt_paths = exempt_paths.clone();
      let trusted_origins = trusted_origins.clone();
      let session_key = session_key.clone();

      Box::pin(async move {
        let path = req.uri().path().to_string();

        // Issue path: safe methods or exempt paths short-circuit verification.
        let safe_method = !is_unsafe_method(req.method());
        let exempt = exempt_paths.iter().any(|p| path.starts_with(p.as_str()));
        if safe_method || exempt {
          let mut resp = next.run(req).await;
          let seed = req_session_token(&resp);
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            &seed,
          );
          return resp;
        }

        // Extract candidate tokens.
        let cookie_token = extract_cookie(&req, &cookie_name).map(str::to_string);
        let header_token = req
          .headers()
          .get(header_name.as_str())
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);
        let session_token = req
          .extensions()
          .get::<Session>()
          .and_then(|s| s.get::<String>(&session_key));

        let cookie_header_match = matches!(
          (cookie_token.as_deref(), header_token.as_deref()),
          (Some(c), Some(h)) if c == h && !c.is_empty()
        );
        let session_match = match (&session_token, &cookie_token) {
          (Some(s), Some(c)) => s == c,
          // No session present → don't enforce session match.
          (None, _) => true,
          _ => false,
        };

        if cookie_header_match && session_match {
          let mut resp = next.run(req).await;
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            &session_token.or(cookie_token),
          );
          return resp;
        }

        // Fallback: trusted Origin / Referer header.
        let trust_hit = if !trusted_origins.is_empty() {
          let origin = req
            .headers()
            .get(http::header::ORIGIN)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
          let referer = req
            .headers()
            .get(http::header::REFERER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
          origin
            .as_deref()
            .map(|o| origin_allowed(o, &trusted_origins))
            .unwrap_or(false)
            || referer
              .as_deref()
              .map(|r| origin_allowed(r, &trusted_origins))
              .unwrap_or(false)
        } else {
          false
        };

        if trust_hit {
          let mut resp = next.run(req).await;
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            &session_token.or(cookie_token),
          );
          return resp;
        }

        (StatusCode::FORBIDDEN, "CSRF token mismatch").into_response()
      })
    }
  }
}

/// When the response originated from a handler that already set a fresh CSRF
/// cookie via `Set-Cookie`, return it so we don't override.
fn req_session_token(resp: &Response) -> Option<String> {
  resp
    .headers()
    .get_all(http::header::SET_COOKIE)
    .iter()
    .filter_map(|v| v.to_str().ok())
    .find_map(|s| {
      let pair = s.split(';').next()?;
      let (name, value) = pair.split_once('=')?;
      if name.trim() == "__csrf_seed" {
        Some(value.trim().to_string())
      } else {
        None
      }
    })
}

fn ensure_csrf_cookie(
  resp: &mut Response,
  cookie_name: &str,
  secure: bool,
  same_site: SameSite,
  session_key: &str,
  bind_to_session: bool,
  preferred_token: &Option<String>,
) {
  let already_set = resp
    .headers()
    .get_all(http::header::SET_COOKIE)
    .iter()
    .any(|v| v.to_str().unwrap_or("").starts_with(cookie_name));
  if already_set {
    return;
  }
  let token = preferred_token
    .clone()
    .unwrap_or_else(generate_csrf_token);
  if bind_to_session {
    if let Some(session) = resp.extensions_mut().get::<Session>().cloned() {
      session.set(session_key, token.clone());
    }
  }
  let cookie = build_cookie(cookie_name, &token, secure, same_site);
  if let Ok(v) = HeaderValue::from_str(&cookie) {
    resp.headers_mut().append(http::header::SET_COOKIE, v);
  }
}
