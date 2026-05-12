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

fn generate_csrf_token() -> String {
  // UUIDv4 carries 122 bits of OS-RNG entropy (6 bits encode version + RFC
  // 4122 variant, the rest is `getrandom`). That is enough for unguessable
  // CSRF tokens but is *not* 128 bits — do not advertise this as
  // 128-bit-secure. If you need a wider random space, swap this for a
  // `rand_core::OsRng` + base64-encoded buffer.
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
  // Match by normalized scheme://host[:port] — lowercase scheme/host, drop
  // default ports, drop any path/query that leaked into the header. The byte-
  // equality version we used previously rejected `https://EXAMPLE.com` vs
  // `https://example.com:443/` even when both should match, and worse, let a
  // case-mismatched allow-list entry bypass the comparison entirely.
  let target = normalize_origin(value);
  if target.is_empty() {
    return false;
  }
  allow.iter().any(|o| normalize_origin(o) == target)
}

/// Normalises an Origin / Referer header (or an allow-list entry) into
/// `scheme://host[:port]` with lowercase scheme + host and default ports
/// dropped. Returns an empty string when parsing fails. Mirrors the helper
/// in `tako-streams::ws` so the two CORS-style checks stay consistent.
fn normalize_origin(raw: &str) -> String {
  let raw = raw.trim();
  if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
    return String::new();
  }
  let Some((scheme, rest)) = raw.split_once("://") else {
    return String::new();
  };
  let scheme = scheme.to_ascii_lowercase();
  let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
  let (host, port) = match authority.rsplit_once(':') {
    Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) && !p.is_empty() => (h, Some(p)),
    _ => (authority, None),
  };
  let host = host.to_ascii_lowercase();
  let default_port = matches!(
    (scheme.as_str(), port),
    ("http", Some("80")) | ("https", Some("443"))
  );
  if let Some(p) = port
    && !default_port
  {
    format!("{scheme}://{host}:{p}")
  } else {
    format!("{scheme}://{host}")
  }
}

fn build_cookie(name: &str, token: &str, secure: bool, same_site: SameSite) -> String {
  let mut s = format!(
    "{}={}; Path=/; SameSite={}",
    name,
    token,
    same_site_str(same_site)
  );
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
        // Snapshot the Session handle BEFORE we hand the request to the
        // downstream handler. Session lives in *request* extensions and gets
        // dropped together with `req`; reading it off `resp.extensions` (the
        // previous implementation) saw nothing, which silently disabled
        // CSRF session-binding entirely.
        let session = req.extensions().get::<Session>().cloned();

        // Issue path: safe methods or exempt paths short-circuit verification.
        let safe_method = !is_unsafe_method(req.method());
        let exempt = exempt_paths.iter().any(|p| path.starts_with(p.as_str()));
        if safe_method || exempt {
          let mut resp = next.run(req).await;
          // If the handler called `Session::rotate()` we must mint a fresh
          // CSRF token to invalidate any stolen pair from the pre-rotation
          // identity. Otherwise a privilege transition (login, role change)
          // would leave the CSRF cookie usable against the new session id.
          let rotated = session.as_ref().is_some_and(|s| s.rotation_requested());
          let seed = if rotated {
            None
          } else {
            req_session_token(&resp)
          };
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            &seed,
            session.as_ref(),
          );
          return resp;
        }

        // When `bind_to_session=true` is configured the only thing standing
        // between the attacker and a successful unsafe request would be the
        // double-submit cookie pattern (cookie == header). An XSS-stolen
        // cookie can be echoed into the matching header trivially, so without
        // a Session the binding is non-existent. Fail closed.
        if bind_to_session && session.is_none() {
          return (
            StatusCode::FORBIDDEN,
            "CSRF: session required for token binding",
          )
            .into_response();
        }

        // Extract candidate tokens.
        let cookie_token = extract_cookie(&req, &cookie_name).map(str::to_string);
        let header_token = req
          .headers()
          .get(header_name.as_str())
          .and_then(|v| v.to_str().ok())
          .map(str::to_string);
        let session_token = session.as_ref().and_then(|s| s.get::<String>(&session_key));

        let cookie_header_match = matches!(
          (cookie_token.as_deref(), header_token.as_deref()),
          (Some(c), Some(h)) if c == h && !c.is_empty()
        );
        let session_match = match (&session_token, &cookie_token) {
          (Some(s), Some(c)) => s == c,
          // No session present and bind_to_session is off → rely on
          // cookie/header double-submit only.
          (None, _) => true,
          _ => false,
        };

        if cookie_header_match && session_match {
          let mut resp = next.run(req).await;
          let rotated = session.as_ref().is_some_and(|s| s.rotation_requested());
          let preferred = if rotated {
            None
          } else {
            session_token.or(cookie_token)
          };
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            &preferred,
            session.as_ref(),
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
          let rotated = session.as_ref().is_some_and(|s| s.rotation_requested());
          let preferred = if rotated {
            None
          } else {
            session_token.or(cookie_token)
          };
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            &preferred,
            session.as_ref(),
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

#[allow(clippy::too_many_arguments)]
fn ensure_csrf_cookie(
  resp: &mut Response,
  cookie_name: &str,
  secure: bool,
  same_site: SameSite,
  session_key: &str,
  bind_to_session: bool,
  preferred_token: &Option<String>,
  session: Option<&Session>,
) {
  let already_set = resp
    .headers()
    .get_all(http::header::SET_COOKIE)
    .iter()
    .any(|v| v.to_str().unwrap_or("").starts_with(cookie_name));
  if already_set {
    return;
  }
  let token = preferred_token.clone().unwrap_or_else(generate_csrf_token);
  if bind_to_session && let Some(session) = session {
    session.set(session_key, token.clone());
  }
  let cookie = build_cookie(cookie_name, &token, secure, same_site);
  if let Ok(v) = HeaderValue::from_str(&cookie) {
    resp.headers_mut().append(http::header::SET_COOKIE, v);
  }
}
