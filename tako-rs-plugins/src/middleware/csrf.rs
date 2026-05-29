//! CSRF protection middleware.
//!
//! Default mode is the **double-submit cookie** pattern: a random token is
//! placed in a cookie *and* must be echoed back in a request header (or form
//! field). The middleware verifies the two values match and that, when a
//! [`Session`] extension is present, the cookie was issued for the current
//! session.
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
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

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
///
/// Uses [`url::Url::parse`] to correctly handle IPv6 literals, userinfo, and
/// trailing paths/queries — the previous string-splitting variant mishandled
/// `https://[::1]:8443` (colon split) and `https://user@example.com`
/// (userinfo leakage into the host comparison).
fn normalize_origin(raw: &str) -> String {
  let raw = raw.trim();
  if raw.is_empty() || raw.eq_ignore_ascii_case("null") {
    return String::new();
  }
  let Ok(url) = url::Url::parse(raw) else {
    return String::new();
  };
  if !url.username().is_empty() || url.password().is_some() {
    return String::new();
  }
  let scheme = url.scheme().to_ascii_lowercase();
  let Some(host) = url.host_str() else {
    return String::new();
  };
  let host = host.to_ascii_lowercase();
  let port = url.port();
  let default = matches!(
    (scheme.as_str(), port),
    ("http" | "ws", Some(80)) | ("https" | "wss", Some(443))
  );
  match port {
    Some(p) if !default => format!("{scheme}://{host}:{p}"),
    _ => format!("{scheme}://{host}"),
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
          let rotated = session
            .as_ref()
            .is_some_and(super::session::Session::rotation_requested);
          let seed = if rotated {
            None
          } else {
            req_session_token(&resp)
          };
          // PMW-12(a): the `__csrf_seed` cookie is an internal handler
          // hook; strip it before the response leaves the server so the
          // marker name never reaches the client.
          if seed.is_some() {
            strip_csrf_seed_cookie(&mut resp);
          }
          ensure_csrf_cookie(
            &mut resp,
            &cookie_name,
            secure,
            same_site,
            &session_key,
            bind_to_session,
            seed.as_ref(),
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
        // PMW-11: with `bind_to_session=true` (default), a session that has
        // not yet been seeded with the CSRF token leaves a bootstrap gap —
        // a stolen cookie (post-rotation, stale window) replayed with a
        // matching header would otherwise pass `session_match` via the
        // `(None, _) => true` arm. Fail-closed in that mode so the only
        // accepted path is "session present AND session token matches
        // cookie". The unbound mode (`bind_to_session=false`) keeps the
        // double-submit-only fallback.
        let session_match = match (&session_token, &cookie_token) {
          (Some(s), Some(c)) => s == c,
          // No session or empty session bucket — only safe under unbound
          // mode where the cookie/header double-submit is the sole guard.
          (None, _) => !bind_to_session,
          _ => false,
        };

        if cookie_header_match && session_match {
          let mut resp = next.run(req).await;
          let rotated = session
            .as_ref()
            .is_some_and(super::session::Session::rotation_requested);
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
            preferred.as_ref(),
            session.as_ref(),
          );
          return resp;
        }

        // Fallback: trusted Origin / Referer header.
        let trust_hit = if trusted_origins.is_empty() {
          false
        } else {
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
            .is_some_and(|o| origin_allowed(o, &trusted_origins))
            || referer
              .as_deref()
              .is_some_and(|r| origin_allowed(r, &trusted_origins))
        };

        if trust_hit {
          let mut resp = next.run(req).await;
          let rotated = session
            .as_ref()
            .is_some_and(super::session::Session::rotation_requested);
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
            preferred.as_ref(),
            session.as_ref(),
          );
          return resp;
        }

        (StatusCode::FORBIDDEN, "CSRF token mismatch").into_response()
      })
    }
  }
}

/// Handler-cooperation hook: a handler that wants to seed the next response's
/// CSRF cookie from its own logic can do so by emitting a one-shot
/// `Set-Cookie: __csrf_seed=<token>` header. The middleware extracts the
/// value here, uses it as the next cookie's payload, and **strips the
/// `__csrf_seed` Set-Cookie line before the response leaves the server** so
/// the marker never reaches the client.
///
/// Typical use case: login handler mints a new session+CSRF pair atomically;
/// it sets the session via the session middleware AND emits `__csrf_seed`
/// so the outgoing CSRF cookie carries the same token that's now bound to
/// the new session, instead of letting the CSRF middleware mint a random
/// one and break the binding.
///
/// Returns `Some(token)` if the marker was found (and signals to the caller
/// that it must be stripped via [`strip_csrf_seed_cookie`]).
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

/// Remove the internal `__csrf_seed` Set-Cookie marker from the outgoing
/// response. Called after [`req_session_token`] consumed the value so the
/// hook stays server-internal and isn't echoed to the browser.
fn strip_csrf_seed_cookie(resp: &mut Response) {
  let headers = resp.headers_mut();
  let kept: Vec<http::HeaderValue> = headers
    .get_all(http::header::SET_COOKIE)
    .iter()
    .filter(|v| {
      let s = v.to_str().unwrap_or("");
      let first = s.split(';').next().unwrap_or("");
      let name = first
        .split_once('=')
        .map_or(first.trim(), |(n, _)| n.trim());
      name != "__csrf_seed"
    })
    .cloned()
    .collect();
  headers.remove(http::header::SET_COOKIE);
  for v in kept {
    headers.append(http::header::SET_COOKIE, v);
  }
}

#[allow(clippy::too_many_arguments)]
fn ensure_csrf_cookie(
  resp: &mut Response,
  cookie_name: &str,
  secure: bool,
  same_site: SameSite,
  session_key: &str,
  bind_to_session: bool,
  preferred_token: Option<&String>,
  session: Option<&Session>,
) {
  // PMW-12(b): the previous `starts_with(cookie_name)` matched any
  // Set-Cookie whose name *began with* `cookie_name`. With cookie_name="csrf"
  // a sibling cookie like `csrf_backup=…` would suppress the CSRF cookie
  // emission entirely. Parse the cookie name out of each Set-Cookie line
  // and compare for exact equality.
  let already_set = resp
    .headers()
    .get_all(http::header::SET_COOKIE)
    .iter()
    .filter_map(|v| v.to_str().ok())
    .any(|s| {
      let first = s.split(';').next().unwrap_or("");
      let name = first
        .split_once('=')
        .map_or(first.trim(), |(n, _)| n.trim());
      name == cookie_name
    });
  if already_set {
    return;
  }
  let token = preferred_token.cloned().unwrap_or_else(generate_csrf_token);
  if bind_to_session && let Some(session) = session {
    session.set(session_key, token.clone());
  }
  let cookie = build_cookie(cookie_name, &token, secure, same_site);
  if let Ok(v) = HeaderValue::from_str(&cookie) {
    resp.headers_mut().append(http::header::SET_COOKIE, v);
  }
}
