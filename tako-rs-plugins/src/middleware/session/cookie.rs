//! Cookie serialization, `SameSite` selection, and session-id generation.

use tako_rs_core::types::Request;

/// `SameSite` cookie attribute.
#[derive(Clone, Copy, Debug)]
pub enum SameSite {
  Strict,
  Lax,
  None,
}

impl SameSite {
  fn as_str(self) -> &'static str {
    match self {
      SameSite::Strict => "Strict",
      SameSite::Lax => "Lax",
      SameSite::None => "None",
    }
  }
}

pub(crate) fn generate_session_id() -> String {
  // UUIDv4 from `getrandom` — 122 bits of unpredictable entropy (the other 6
  // bits encode the RFC 4122 version + variant). Plenty for session-cookie
  // unguessability, but document the bit count rather than claim "128 bits"
  // since that misconception leaks into security reviews.
  uuid::Uuid::new_v4().simple().to_string()
}

pub(crate) fn extract_cookie_value<'a>(req: &'a Request, cookie_name: &str) -> Option<&'a str> {
  req
    .headers()
    .get(http::header::COOKIE)
    .and_then(|v| v.to_str().ok())
    .and_then(|cookies| {
      cookies.split(';').find_map(|pair| {
        let pair = pair.trim();
        let (name, value) = pair.split_once('=')?;
        if name.trim() == cookie_name {
          Some(value.trim())
        } else {
          None
        }
      })
    })
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_cookie(
  cookie_name: &str,
  sid: &str,
  path: &str,
  domain: Option<&str>,
  ttl_secs: u64,
  secure: bool,
  http_only: bool,
  same_site: SameSite,
) -> String {
  let mut s = format!("{cookie_name}={sid}; Path={path}");
  if let Some(d) = domain {
    s.push_str("; Domain=");
    s.push_str(d);
  }
  s.push_str(&format!("; Max-Age={ttl_secs}"));
  if http_only {
    s.push_str("; HttpOnly");
  }
  if secure {
    s.push_str("; Secure");
  }
  s.push_str("; SameSite=");
  s.push_str(same_site.as_str());
  s
}

pub(crate) fn build_expired_cookie(
  cookie_name: &str,
  path: &str,
  domain: Option<&str>,
  secure: bool,
  http_only: bool,
  same_site: SameSite,
) -> String {
  // Empty value + Max-Age=0 + far-past Expires covers every major UA
  // (some only honor one of the two attributes).
  let mut s = format!("{cookie_name}=; Path={path}");
  if let Some(d) = domain {
    s.push_str("; Domain=");
    s.push_str(d);
  }
  s.push_str("; Max-Age=0; Expires=Thu, 01 Jan 1970 00:00:00 GMT");
  if http_only {
    s.push_str("; HttpOnly");
  }
  if secure {
    s.push_str("; Secure");
  }
  s.push_str("; SameSite=");
  s.push_str(same_site.as_str());
  s
}
