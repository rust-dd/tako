//! URI-derived extractors: `OriginalUri`, `Host`, `Scheme`.
//!
//! These mirror axum and surface request location data from `Uri`, `Host`
//! header, and the underlying transport. `OriginalUri` is captured on first
//! observation so middleware that mutates the request URI (e.g. `nest` /
//! `scope` rewrites) cannot lose the original.

use http::StatusCode;
use http::Uri;
use http::request::Parts;
use tako_core::extractors::FromRequest;
use tako_core::extractors::FromRequestParts;
use tako_core::responder::Responder;
use tako_core::types::Request;

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
      .map(|m| m.0.clone())
      .unwrap_or_else(|| req.uri().clone());
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
      .map(|m| m.0.clone())
      .unwrap_or_else(|| parts.uri.clone());
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

fn extract_host(headers: &http::HeaderMap, uri: &Uri) -> Option<String> {
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
    futures_util::future::ready(
      extract_host(req.headers(), req.uri())
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
    futures_util::future::ready(
      extract_host(&parts.headers, &parts.uri)
        .map(Host)
        .ok_or(HostMissing),
    )
  }
}

/// HTTP scheme derived from the request URI / `X-Forwarded-Proto` /
/// transport-injected `ConnInfo`.
pub struct Scheme(pub String);

fn extract_scheme(headers: &http::HeaderMap, uri: &Uri, ext: &http::Extensions) -> String {
  if let Some(p) = headers
    .get("x-forwarded-proto")
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.split(',').next())
    .map(|s| s.trim())
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
    let scheme = extract_scheme(req.headers(), req.uri(), req.extensions());
    futures_util::future::ready(Ok(Scheme(scheme)))
  }
}

impl<'a> FromRequestParts<'a> for Scheme {
  type Error = std::convert::Infallible;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let scheme = extract_scheme(&parts.headers, &parts.uri, &parts.extensions);
    futures_util::future::ready(Ok(Scheme(scheme)))
  }
}
