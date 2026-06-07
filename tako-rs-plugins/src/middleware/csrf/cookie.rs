//! Response-side cookie emission and the handler-cooperation seed hook.

use http::HeaderValue;
use tako_rs_core::types::Response;

use super::token::build_cookie;
use super::token::generate_csrf_token;
use crate::middleware::session::SameSite;
use crate::middleware::session::Session;

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
pub(crate) fn req_session_token(resp: &Response) -> Option<String> {
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
pub(crate) fn strip_csrf_seed_cookie(resp: &mut Response) {
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
pub(crate) fn ensure_csrf_cookie(
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
