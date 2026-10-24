//! `OriginalUri`, `Host` and `Scheme`; the last two trust forwarding headers only from a peer
//! listed in a [`UriPartsConfig`].

use std::borrow::Cow;
use std::net::IpAddr;

use http::StatusCode;
use http::Uri;
use http::request::Parts;
use http::uri::Authority;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// Proxies whose forwarding headers `Host` and `Scheme` honor; read from a request extension,
/// then `Router::with_state`, then a process-global value from `get_or_init_state`.
#[derive(Debug, Clone, Default)]
pub struct UriPartsConfig {
  /// Direct-peer IPs; empty trusts no forwarding headers.
  pub trusted_proxies: Vec<IpAddr>,
}

impl UriPartsConfig {
  pub fn with_trusted_proxy(mut self, ip: IpAddr) -> Self {
    self.trusted_proxies.push(ip);
    self
  }
}

fn lookup_uri_parts_cfg(ext: &http::Extensions) -> Option<UriPartsConfig> {
  if let Some(cfg) = ext.get::<UriPartsConfig>() {
    return Some(cfg.clone());
  }
  if let Some(rs) = ext.get::<std::sync::Arc<tako_rs_core::router_state::RouterState>>()
    && let Some(arc) = rs.get::<UriPartsConfig>()
  {
    return Some((*arc).clone());
  }
  tako_rs_core::state::get_state::<UriPartsConfig>().map(|arc| (*arc).clone())
}

fn peer_is_trusted(ext: &http::Extensions) -> bool {
  let Some(cfg) = lookup_uri_parts_cfg(ext) else {
    return false;
  };
  if cfg.trusted_proxies.is_empty() {
    return false;
  }
  let peer_ip = ext
    .get::<tako_rs_core::conn_info::ConnInfo>()
    .and_then(|info| match &info.peer {
      tako_rs_core::conn_info::PeerAddr::Ip(sa) => Some(sa.ip()),
      _ => None,
    });
  match peer_ip {
    Some(ip) => cfg.trusted_proxies.contains(&ip),
    None => false,
  }
}

pub use tako_rs_core::router_state::OriginalUriMarker;

/// The URI saved at router entry, before middleware or nested dispatch rewrites it;
/// falls back to the request URI when extracted without a router.
pub struct OriginalUri(pub Uri);

impl<'a> FromRequest<'a> for OriginalUri {
  type Error = std::convert::Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let uri = req
      .extensions()
      .get::<OriginalUriMarker>()
      .map_or_else(|| req.uri().clone(), |m| m.0.clone());
    futures_util::future::ready(Ok(OriginalUri(uri)))
  }
}

impl<'a> FromRequestParts<'a> for OriginalUri {
  type Error = std::convert::Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let uri = parts
      .extensions
      .get::<OriginalUriMarker>()
      .map_or_else(|| parts.uri.clone(), |m| m.0.clone());
    futures_util::future::ready(Ok(OriginalUri(uri)))
  }
}

/// The `Host` header, else the URI authority, with a trusted proxy's `Forwarded: host=` and
/// `X-Forwarded-Host` taking precedence. Missing, malformed, or conflicting Host/authority
/// values reject with 400, including when forwarding headers are trusted.
pub struct Host(pub String);

#[derive(Debug)]
pub struct HostMissing;

impl Responder for HostMissing {
  fn into_response(self) -> tako_rs_core::types::Response {
    (
      StatusCode::BAD_REQUEST,
      "request has no valid, unambiguous host",
    )
      .into_response()
  }
}

fn extract_host(headers: &http::HeaderMap, uri: &Uri, trust_forwarded: bool) -> Option<String> {
  let mut hosts = headers.get_all(http::header::HOST).iter();
  let host = match hosts.next() {
    Some(value) => Some(valid_authority(value.to_str().ok()?)?),
    None => None,
  };
  if hosts.next().is_some() {
    return None;
  }
  let authority = match uri.authority() {
    Some(value) => Some(valid_authority(value.as_str())?),
    None => None,
  };
  if let (Some(host), Some(authority)) = (&host, &authority) {
    let default_port = match uri.scheme_str() {
      Some("https") => Some(443),
      Some("http") => Some(80),
      _ => None,
    };
    if !host.host().eq_ignore_ascii_case(authority.host())
      || host.port_u16().or(default_port) != authority.port_u16().or(default_port)
    {
      return None;
    }
  }
  if trust_forwarded {
    if let Some(forwarded) = headers.get("forwarded").and_then(|v| v.to_str().ok()) {
      let (first, _) = split_forwarded(forwarded, b',')?;
      let mut remaining = Some(first);
      while let Some(parameters) = remaining {
        let (pair, rest) = split_forwarded(parameters, b';')?;
        remaining = rest;
        if let Some((name, value)) = pair.trim().split_once('=')
          && name.eq_ignore_ascii_case("host")
        {
          let value = forwarded_value(value.trim())?;
          return valid_authority(&value).map(|a| a.to_string());
        }
      }
    }
    if let Some(xfh) = headers
      .get("x-forwarded-host")
      .and_then(|v| v.to_str().ok())
      .and_then(|v| v.split(',').next())
    {
      return valid_authority(xfh.trim()).map(|a| a.to_string());
    }
  }
  host.or(authority).map(|a| a.to_string())
}

fn split_forwarded(value: &str, separator: u8) -> Option<(&str, Option<&str>)> {
  let mut quoted = false;
  let mut escaped = false;
  for (index, byte) in value.bytes().enumerate() {
    if escaped {
      escaped = false;
    } else if quoted && byte == b'\\' {
      escaped = true;
    } else if byte == b'"' {
      quoted = !quoted;
    } else if !quoted && byte == separator {
      return Some((&value[..index], Some(&value[index + 1..])));
    }
  }
  (!quoted && !escaped).then_some((value, None))
}

fn forwarded_value(value: &str) -> Option<Cow<'_, str>> {
  if !value.starts_with('"') {
    return Some(Cow::Borrowed(value));
  }
  let quoted = value.strip_prefix('"')?.strip_suffix('"')?;
  if !quoted.contains('\\') {
    return Some(Cow::Borrowed(quoted));
  }
  let mut decoded = String::with_capacity(quoted.len());
  let mut chars = quoted.chars();
  while let Some(character) = chars.next() {
    decoded.push(if character == '\\' {
      chars.next()?
    } else {
      character
    });
  }
  Some(Cow::Owned(decoded))
}

fn valid_authority(value: &str) -> Option<Authority> {
  let authority = value.parse::<Authority>().ok()?;
  if value.contains('@') || authority.host().is_empty() {
    return None;
  }
  let suffix = authority.as_str().strip_prefix(authority.host())?;
  if !suffix.is_empty() && (!suffix.starts_with(':') || authority.port_u16().is_none()) {
    return None;
  }
  Some(authority)
}

impl<'a> FromRequest<'a> for Host {
  type Error = HostMissing;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let trust = peer_is_trusted(req.extensions());
    futures_util::future::ready(
      extract_host(req.headers(), req.uri(), trust)
        .map(Host)
        .ok_or(HostMissing),
    )
  }
}

impl<'a> FromRequestParts<'a> for Host {
  type Error = HostMissing;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let trust = peer_is_trusted(&parts.extensions);
    futures_util::future::ready(
      extract_host(&parts.headers, &parts.uri, trust)
        .map(Host)
        .ok_or(HostMissing),
    )
  }
}

/// The lowercase scheme: a trusted proxy's `X-Forwarded-Proto`, else the URI's, else `https`
/// on TLS connections and `http` otherwise.
pub struct Scheme(pub String);

fn extract_scheme(
  headers: &http::HeaderMap,
  uri: &Uri,
  ext: &http::Extensions,
  trust_forwarded: bool,
) -> String {
  if trust_forwarded
    && let Some(p) = headers
      .get("x-forwarded-proto")
      .and_then(|v| v.to_str().ok())
      .and_then(|v| v.split(',').next())
      .map(str::trim)
    && !p.is_empty()
  {
    return p.to_ascii_lowercase();
  }
  if let Some(s) = uri.scheme_str() {
    return s.to_ascii_lowercase();
  }
  if let Some(info) = ext.get::<tako_rs_core::conn_info::ConnInfo>()
    && info.tls.is_some()
  {
    return "https".into();
  }
  "http".into()
}

impl<'a> FromRequest<'a> for Scheme {
  type Error = std::convert::Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let trust = peer_is_trusted(req.extensions());
    let scheme = extract_scheme(req.headers(), req.uri(), req.extensions(), trust);
    futures_util::future::ready(Ok(Scheme(scheme)))
  }
}

impl<'a> FromRequestParts<'a> for Scheme {
  type Error = std::convert::Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let trust = peer_is_trusted(&parts.extensions);
    let scheme = extract_scheme(&parts.headers, &parts.uri, &parts.extensions, trust);
    futures_util::future::ready(Ok(Scheme(scheme)))
  }
}
