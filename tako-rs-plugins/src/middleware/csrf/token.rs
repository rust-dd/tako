use http::Method;
use http::header;
use tako_rs_core::types::Request;

use crate::middleware::session::SameSite;

pub(crate) fn generate_csrf_token() -> String {
  // 122 random bits: unguessable, but do not advertise it as 128-bit secure.
  uuid::Uuid::new_v4().simple().to_string()
}

pub(crate) fn is_unsafe_method(method: &Method) -> bool {
  matches!(
    *method,
    Method::POST | Method::PUT | Method::DELETE | Method::PATCH
  )
}

pub(crate) fn path_is_exempt(path: &str, exemptions: &[String]) -> bool {
  exemptions.iter().any(|prefix| {
    let prefix = prefix.trim_end_matches('/');
    path == prefix
      || path
        .strip_prefix(prefix)
        .is_some_and(|suffix| suffix.starts_with('/'))
  })
}

/// Whether `Origin` or `Referer` names an allow-listed origin.
pub(crate) fn origin_trusted(req: &Request, allow: &[String]) -> bool {
  !allow.is_empty()
    && [header::ORIGIN, header::REFERER].iter().any(|name| {
      req
        .headers()
        .get(name)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| origin_allowed(value, allow))
    })
}

fn origin_allowed(value: &str, allow: &[String]) -> bool {
  let target = normalize_origin(value);
  !target.is_empty() && allow.iter().any(|o| normalize_origin(o) == target)
}

/// `scheme://host[:port]` with the host lowercased; empty when unparsable or carrying
/// credentials. `Url::port` already drops default ports.
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
  let scheme = url.scheme();
  let Some(host) = url.host_str() else {
    return String::new();
  };
  let host = host.to_ascii_lowercase();
  match url.port() {
    Some(port) => format!("{scheme}://{host}:{port}"),
    None => format!("{scheme}://{host}"),
  }
}

pub(crate) fn build_cookie(name: &str, token: &str, secure: bool, same_site: SameSite) -> String {
  let mut s = format!(
    "{}={}; Path=/; SameSite={}",
    name,
    token,
    same_site.as_str()
  );
  if secure || matches!(same_site, SameSite::None) {
    s.push_str("; Secure");
  }
  s
}
