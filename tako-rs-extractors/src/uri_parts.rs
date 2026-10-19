//! `OriginalUri`, `Host` and `Scheme`; the last two trust forwarding headers only from a peer
//! listed in a [`UriPartsConfig`].

use std::net::IpAddr;

use http::StatusCode;
use http::Uri;
use http::request::Parts;
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
/// `X-Forwarded-Host` taking precedence; rejects with 400 when none is present.
pub struct Host(pub String);

#[derive(Debug)]
pub struct HostMissing;

impl Responder for HostMissing {
  fn into_response(self) -> tako_rs_core::types::Response {
    (StatusCode::BAD_REQUEST, "request has no host").into_response()
  }
}

fn extract_host(headers: &http::HeaderMap, uri: &Uri, trust_forwarded: bool) -> Option<String> {
  if trust_forwarded {
    if let Some(forwarded) = headers.get("forwarded").and_then(|v| v.to_str().ok()) {
      for pair in forwarded.split(';') {
        let pair = pair.trim();
        if let Some(rest) = pair.strip_prefix("host=") {
          let host = rest.trim_matches('"');
          if !host.is_empty() {
            return Some(host.to_string());
          }
        }
      }
    }
    if let Some(xfh) = headers
      .get("x-forwarded-host")
      .and_then(|v| v.to_str().ok())
      .and_then(|v| v.split(',').next())
      .map(|s| s.trim().to_string())
      && !xfh.is_empty()
    {
      return Some(xfh);
    }
  }
  if let Some(host) = headers
    .get(http::header::HOST)
    .and_then(|v| v.to_str().ok())
  {
    return Some(host.to_string());
  }
  uri.authority().map(|a| a.as_str().to_string())
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
