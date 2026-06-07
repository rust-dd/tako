//! Token generation and request-side verification primitives.

use http::Method;
use tako_rs_core::types::Request;

use crate::middleware::session::SameSite;

pub(crate) fn generate_csrf_token() -> String {
  // UUIDv4 carries 122 bits of OS-RNG entropy (6 bits encode version + RFC
  // 4122 variant, the rest is `getrandom`). That is enough for unguessable
  // CSRF tokens but is *not* 128 bits — do not advertise this as
  // 128-bit-secure. If you need a wider random space, swap this for a
  // `rand_core::OsRng` + base64-encoded buffer.
  uuid::Uuid::new_v4().simple().to_string()
}

pub(crate) fn is_unsafe_method(method: &Method) -> bool {
  matches!(
    *method,
    Method::POST | Method::PUT | Method::DELETE | Method::PATCH
  )
}

pub(crate) fn extract_cookie<'a>(req: &'a Request, name: &str) -> Option<&'a str> {
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

pub(crate) fn origin_allowed(value: &str, allow: &[String]) -> bool {
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

pub(crate) fn build_cookie(name: &str, token: &str, secure: bool, same_site: SameSite) -> String {
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
