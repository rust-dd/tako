//! Client IP address extraction from HTTP request headers.
//!
//! This module provides the [`IpAddr`](crate::ipaddr::IpAddr) extractor for determining the client's IP address
//! from various HTTP headers commonly used by proxies, load balancers, and CDNs.
//! It supports both IPv4 and IPv6 addresses and provides methods for inspecting
//! IP address properties like whether it's private, loopback, etc.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::ipaddr::IpAddr;
//! use std::net::IpAddr as StdIpAddr;
//!
//! async fn handle_request(ip: IpAddr) {
//!     println!("Client IP: {}", ip);
//!
//!     if ip.is_private() {
//!         println!("Request from private network");
//!     }
//!
//!     if ip.is_ipv4() {
//!         println!("IPv4 address");
//!     } else {
//!         println!("IPv6 address");
//!     }
//! }
//! ```

use std::net::IpAddr as StdIpAddr;
use std::net::SocketAddr;
use std::str::FromStr;

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// Extractor for the client IP address.
///
/// **Default behavior (secure):** Returns the transport-level peer IP from
/// `ConnInfo` (or the legacy `SocketAddr` extension). Forwarded headers
/// (`X-Forwarded-For`, `X-Real-IP`, `Forwarded`, …) are **ignored** because
/// any client that can reach the server directly can forge them.
///
/// **Trusted-proxy mode:** Insert an [`IpAddrConfig`] into router state via
/// `tako_rs_core::state::set_state` with `trusted_proxies` listing the IPs of
/// your real proxy/load-balancer fleet. When the direct peer matches one of
/// those entries, forwarded headers are honored in priority order:
/// 1. `Forwarded` (RFC 7239 — `for=`)
/// 2. `X-Forwarded-For` (leftmost untrusted hop)
/// 3. `X-Real-IP`
/// 4. `X-Client-IP`
/// 5. `CF-Connecting-IP` (Cloudflare)
/// 6. `True-Client-IP`
///
/// # Examples
///
/// ```rust
/// use tako::extractors::ipaddr::IpAddr;
/// use std::net::IpAddr as StdIpAddr;
///
/// let ip = IpAddr::new("192.168.1.1".parse().unwrap());
/// assert!(ip.is_ipv4());
/// assert!(ip.is_private());
/// ```
#[derive(Debug, Clone, PartialEq)]
#[doc(alias = "ip")]
#[doc(alias = "ipaddr")]
pub struct IpAddr(pub StdIpAddr);

/// Configuration for trusted-proxy IP extraction. Insert into router state to
/// opt into forwarded-header parsing for requests whose direct peer matches.
#[derive(Debug, Clone, Default)]
pub struct IpAddrConfig {
  /// Direct-peer IPs whose forwarded-IP headers we honor. Empty (default)
  /// means no header trust — only the direct peer IP is used.
  pub trusted_proxies: Vec<StdIpAddr>,
}

impl IpAddrConfig {
  /// Empty config — no forwarded-header trust.
  pub fn new() -> Self {
    Self::default()
  }

  /// Add a trusted proxy IP.
  pub fn trust(mut self, ip: StdIpAddr) -> Self {
    self.trusted_proxies.push(ip);
    self
  }

  /// Replace the trusted-proxy list.
  pub fn with_trusted_proxies(mut self, ips: Vec<StdIpAddr>) -> Self {
    self.trusted_proxies = ips;
    self
  }
}

/// Error type for IP address extraction.
#[derive(Debug)]
pub enum IpAddrError {
  /// No valid IP address found in any of the checked headers.
  NoIpFound,
  /// The IP address format in the header is invalid.
  InvalidIpFormat(String),
  /// Failed to parse the IP address from the header value.
  HeaderParseError,
}

impl Responder for IpAddrError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      IpAddrError::NoIpFound => (
        StatusCode::BAD_REQUEST,
        "No valid IP address found in request headers",
      )
        .into_response(),
      IpAddrError::InvalidIpFormat(ip) => (
        StatusCode::BAD_REQUEST,
        format!("Invalid IP address format: {ip}"),
      )
        .into_response(),
      IpAddrError::HeaderParseError => (
        StatusCode::BAD_REQUEST,
        "Failed to parse IP address from headers",
      )
        .into_response(),
    }
  }
}

impl IpAddr {
  /// Creates a new `IpAddr` wrapper.
  pub fn new(addr: StdIpAddr) -> Self {
    Self(addr)
  }

  /// Gets the inner IP address.
  pub fn inner(&self) -> StdIpAddr {
    self.0
  }

  /// Checks if the IP address is IPv4.
  pub fn is_ipv4(&self) -> bool {
    self.0.is_ipv4()
  }

  /// Checks if the IP address is IPv6.
  pub fn is_ipv6(&self) -> bool {
    self.0.is_ipv6()
  }

  /// Checks if the IP address is a loopback address.
  pub fn is_loopback(&self) -> bool {
    self.0.is_loopback()
  }

  /// Checks if the IP address is a private address.
  ///
  /// For IPv4, this includes addresses in the ranges:
  /// - 10.0.0.0/8
  /// - 172.16.0.0/12
  /// - 192.168.0.0/16
  /// - 127.0.0.0/8 (loopback)
  ///
  /// For IPv6, this includes:
  /// - `fc00::/7` (Unique Local Addresses)
  /// - `fe80::/10` (Link-Local Addresses)
  /// - `::1` (loopback)
  pub fn is_private(&self) -> bool {
    match self.0 {
      StdIpAddr::V4(ipv4) => ipv4.is_private(),
      StdIpAddr::V6(ipv6) => {
        // IPv6 private address ranges
        let segments = ipv6.segments();
        // fc00::/7 (Unique Local Addresses)
        (segments[0] & 0xfe00) == 0xfc00 ||
                // fe80::/10 (Link-Local Addresses)
                (segments[0] & 0xffc0) == 0xfe80 ||
                // ::1 (Loopback)
                ipv6.is_loopback()
      }
    }
  }

  /// Resolves the client IP from request extensions + headers using the
  /// configured trust policy. Secure-by-default: forwarded headers are only
  /// honored when the direct peer is listed in `IpAddrConfig::trusted_proxies`.
  fn extract_from(
    extensions: &http::Extensions,
    headers: &http::HeaderMap,
  ) -> Result<Self, IpAddrError> {
    let peer = peer_ip_from_extensions(extensions);

    let cfg = tako_rs_core::state::get_state::<IpAddrConfig>();
    let trust_headers = match (peer.as_ref(), cfg.as_ref()) {
      (Some(p), Some(cfg)) => cfg.trusted_proxies.iter().any(|t| t == p),
      _ => false,
    };

    if trust_headers
      && let Some(cfg) = cfg.as_ref()
      && let Some(ip) = Self::parse_forwarded_headers(headers, &cfg.trusted_proxies)
    {
      return Ok(Self(ip));
    }

    peer.map(Self).ok_or(IpAddrError::NoIpFound)
  }

  /// Parses the first non-trusted client IP from any of the recognized
  /// forwarded headers, in priority order.
  ///
  /// For multi-hop headers (`Forwarded`, `X-Forwarded-For`) the walk goes
  /// **right-to-left**, skipping entries that match `trusted_proxies` — the
  /// first remaining entry is the leftmost untrusted hop (the real client).
  /// Walking left-to-right was spoofable: an attacker could prepend a fake
  /// `<spoofed>` to the header and a trusted proxy would append the real
  /// `<peer>`, leaving the first parseable IP as `<spoofed>`.
  ///
  /// Single-IP headers (`X-Real-IP`, `CF-Connecting-IP`, …) carry one
  /// already-resolved client IP from the proxy and are taken as-is.
  fn parse_forwarded_headers(
    headers: &http::HeaderMap,
    trusted_proxies: &[StdIpAddr],
  ) -> Option<StdIpAddr> {
    const MULTI_HOP: &[&str] = &["forwarded", "x-forwarded-for"];
    const SINGLE_HOP: &[&str] = &[
      "x-real-ip",
      "x-client-ip",
      "cf-connecting-ip",
      "true-client-ip",
    ];
    for header_name in MULTI_HOP {
      if let Some(v) = headers.get(*header_name)
        && let Ok(s) = v.to_str()
        && let Some(ip) = Self::parse_ip_right_to_left(s, trusted_proxies)
      {
        return Some(ip);
      }
    }
    for header_name in SINGLE_HOP {
      if let Some(v) = headers.get(*header_name)
        && let Ok(s) = v.to_str()
        && let Some(ip) = Self::parse_ip_from_header(s)
      {
        return Some(ip);
      }
    }
    None
  }

  /// Walk a comma-separated header from right to left and return the first
  /// IP that is not in `trusted_proxies`. Used for multi-hop headers where
  /// the client appends to the left and proxies append to the right.
  fn parse_ip_right_to_left(
    header_value: &str,
    trusted_proxies: &[StdIpAddr],
  ) -> Option<StdIpAddr> {
    let parts: Vec<&str> = header_value.split(',').collect();
    for part in parts.iter().rev() {
      let trimmed = part.trim();
      if trimmed.is_empty() {
        continue;
      }
      // An unparseable entry in the middle of the chain is not a stop
      // condition — it's typically a missing-port or quoted-form variant
      // we don't recognize yet; keep walking left.
      let Some(ip) = Self::parse_ip_from_part(trimmed) else {
        continue;
      };
      if !trusted_proxies.contains(&ip) {
        return Some(ip);
      }
    }
    None
  }

  /// Parses an IP address from a header value (comma-separated list, optional
  /// `for=` prefix, optional `:port` or `[v6]:port` suffix).
  fn parse_ip_from_header(header_value: &str) -> Option<StdIpAddr> {
    for part in header_value.split(',') {
      let part = part.trim();
      if part.is_empty() {
        continue;
      }
      if let Some(ip) = Self::parse_ip_from_part(part) {
        return Some(ip);
      }
    }
    None
  }

  /// Parse one comma-separated entry into an IP, stripping `for=`, quotes,
  /// `[v6]` brackets, and an optional `:port` suffix.
  fn parse_ip_from_part(part: &str) -> Option<StdIpAddr> {
    if part.is_empty() {
      return None;
    }
    let ip_part = part.strip_prefix("for=").unwrap_or(part);
    let ip_part = ip_part.trim_matches('"');

    let ip_str = if ip_part.starts_with('[') {
      if let Some(end) = ip_part.find(']') {
        &ip_part[1..end]
      } else {
        ip_part
      }
    } else if ip_part.matches(':').count() == 1 {
      ip_part.split(':').next().unwrap_or(ip_part)
    } else {
      ip_part
    };

    StdIpAddr::from_str(ip_str).ok()
  }
}

fn peer_ip_from_extensions(ext: &http::Extensions) -> Option<StdIpAddr> {
  if let Some(info) = ext.get::<ConnInfo>()
    && let PeerAddr::Ip(sa) = &info.peer
  {
    return Some(sa.ip());
  }
  if let Some(sa) = ext.get::<SocketAddr>() {
    return Some(sa.ip());
  }
  None
}

impl std::fmt::Display for IpAddr {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "{}", self.0)
  }
}

impl From<StdIpAddr> for IpAddr {
  fn from(addr: StdIpAddr) -> Self {
    Self(addr)
  }
}

impl From<IpAddr> for StdIpAddr {
  fn from(addr: IpAddr) -> Self {
    addr.0
  }
}

impl<'a> FromRequest<'a> for IpAddr {
  type Error = IpAddrError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from(req.extensions(), req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for IpAddr {
  type Error = IpAddrError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from(&parts.extensions, &parts.headers))
  }
}
