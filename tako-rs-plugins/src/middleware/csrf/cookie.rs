use http::HeaderValue;
use tako_rs_core::types::Response;

use super::token::build_cookie;
use super::token::generate_csrf_token;
use crate::middleware::session::SameSite;
use crate::middleware::session::Session;

/// A handler pins the next CSRF token (e.g. at login) with a `__csrf_seed` Set-Cookie;
/// [`ensure_csrf_cookie`] removes the marker before the response is sent.
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

fn set_cookie_name(line: &str) -> &str {
  let first = line.split(';').next().unwrap_or("");
  first
    .split_once('=')
    .map_or(first.trim(), |(name, _)| name.trim())
}

pub(crate) fn strip_csrf_seed_cookie(resp: &mut Response) {
  let headers = resp.headers_mut();
  let kept: Vec<http::HeaderValue> = headers
    .get_all(http::header::SET_COOKIE)
    .iter()
    .filter(|v| set_cookie_name(v.to_str().unwrap_or("")) != "__csrf_seed")
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
  strip_csrf_seed_cookie(resp);
  let already_set = resp
    .headers()
    .get_all(http::header::SET_COOKIE)
    .iter()
    .filter_map(|v| v.to_str().ok())
    .any(|s| set_cookie_name(s) == cookie_name);
  if already_set {
    return;
  }
  let token = preferred_token.cloned().unwrap_or_else(generate_csrf_token);
  if bind_to_session && let Some(session) = session {
    session.set(session_key, token.clone());
  }
  let cookie = build_cookie(cookie_name, &token, secure, same_site);
  if let Ok(v) = HeaderValue::from_str(&cookie) {
    if bind_to_session && let Some(session) = session {
      session.defer_cookie(v);
    } else {
      resp.headers_mut().append(http::header::SET_COOKIE, v);
    }
  }
}
