//! URI-derived extractors: `OriginalUri`, `Host`, `Scheme`.
//!
//! These mirror axum and surface request location data from `Uri`, `Host`
//! header, and the underlying transport. `OriginalUri` is captured on first
//! observation so middleware that mutates the request URI (e.g. `nest` /
//! `scope` rewrites) cannot lose the original.
//!
//! ## Trust model
//!
//! `Host` and `Scheme` honor `Forwarded` / `X-Forwarded-*` headers **only** when
//! the connection's peer IP appears in a configured trusted-proxy list. Insert
//! a `UriPartsConfig` (see [`UriPartsConfig`](crate::uri_parts::UriPartsConfig))
//! into the application state (or request extensions) to enable that path. Without configuration the extractors fall back to the
//! `Host` header and the transport-resolved scheme — never an attacker-supplied
//! `X-Forwarded-Host`, which previously enabled cache-poisoning and open
//! redirect classes when the server was directly reachable.

use std::net::IpAddr;

use http::StatusCode;
use http::Uri;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

/// Configuration governing which forwarding headers `Host` / `Scheme` are
/// allowed to consult. The extractors only honor `Forwarded` /
/// `X-Forwarded-Host` / `X-Forwarded-Proto` when the peer address (from the
/// transport's `ConnInfo`) appears in [`Self::trusted_proxies`].
#[derive(Debug, Clone, Default)]
pub struct UriPartsConfig {
  /// Peer addresses (immediate TCP/UDS counter-party) whose forwarded
  /// headers may be trusted. Empty means "trust no forwarded headers".
  pub trusted_proxies: Vec<IpAddr>,
}

impl UriPartsConfig {
  /// Builder convenience for the common "trust this single proxy" setup.
  pub fn with_trusted_proxy(mut self, ip: IpAddr) -> Self {
    self.trusted_proxies.push(ip);
    self
  }
}

/// Resolve a `UriPartsConfig` consulting (in order) request extensions,
/// per-router state, then process-global state.
///
/// EXT-3: the module docs advertise the config can live in "application state
/// (or request extensions)", but the original implementation only checked
/// extensions. Trusted-proxy config set via `Router::with_state` or
/// `tako_core::state::set_state` was silently inactive — `peer_is_trusted`
/// always returned false, fail-safe in the security sense but a documented
/// feature that did not work. Falls back through both state layers now, with
/// a tiny ~2-3 extra hash probes (scc, lock-free) per Host/Scheme extraction
/// in the no-config-in-extensions case — cold-path enough that it does not
/// move hot-path numbers.
fn lookup_uri_parts_cfg(ext: &http::Extensions) -> Option<UriPartsConfig> {
  if let Some(cfg) = ext.get::<UriPartsConfig>() {
    return Some(cfg.clone());
  }
  if let Some(rs) = ext.get::<std::sync::Arc<tako_core::router_state::RouterState>>()
    && let Some(arc) = rs.get::<UriPartsConfig>()
  {
    return Some((*arc).clone());
  }
  tako_core::state::get_state::<UriPartsConfig>().map(|arc| (*arc).clone())
}

fn peer_is_trusted(ext: &http::Extensions) -> bool {
  let Some(cfg) = lookup_uri_parts_cfg(ext) else {
    return false;
  };
  if cfg.trusted_proxies.is_empty() {
    return false;
  }
  let peer_ip = ext
    .get::<tako_core::conn_info::ConnInfo>()
    .and_then(|info| match &info.peer {
      tako_core::conn_info::PeerAddr::Ip(sa) => Some(sa.ip()),
      _ => None,
    });
  match peer_ip {
    Some(ip) => cfg.trusted_proxies.contains(&ip),
    None => false,
  }
}

/// Marker stored in request extensions to preserve the URI as it first
/// arrived at the dispatcher. `OriginalUri` reads from this.
#[derive(Debug, Clone)]
pub struct OriginalUriMarker(pub Uri);

/// The URI of the request as it first hit the dispatcher.
///
/// Useful when downstream middleware rewrites the URI (nest / scope strip
/// the prefix) and the handler still needs the original path.
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

/// Host derived from `:authority` / `Host` / `X-Forwarded-Host`.
///
/// Resolution order (matches axum):
/// 1. `Forwarded: host=…` (RFC 7239), first entry
/// 2. `X-Forwarded-Host` first value
/// 3. `Host` header
/// 4. `Uri::authority()` (HTTP/2+ when the request line carries authority)
pub struct Host(pub String);

/// Rejection when the host cannot be determined.
#[derive(Debug)]
pub struct HostMissing;

impl Responder for HostMissing {
  fn into_response(self) -> tako_core::types::Response {
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

/// HTTP scheme derived from the request URI / `X-Forwarded-Proto` /
/// transport-injected `ConnInfo`.
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
  if let Some(info) = ext.get::<tako_core::conn_info::ConnInfo>()
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
