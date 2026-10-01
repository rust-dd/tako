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
use std::sync::Arc;

use http::StatusCode;
use http::request::Parts;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::extractors::Entries;
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
/// `Router::with_state` with `trusted_proxies` listing the IPs of
/// your real proxy/load-balancer fleet. When the direct peer matches one of
/// those entries, forwarded headers are honored in priority order:
/// 1. `Forwarded` (RFC 7239 — `for=`)
/// 2. `X-Forwarded-For` (nearest untrusted hop)
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
  /// Networks whose forwarding headers may be trusted.
  pub trusted_networks: Vec<ipnet::IpNet>,
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

  /// Adds a trusted proxy network.
  pub fn trust_network(mut self, network: ipnet::IpNet) -> Self {
    self.trusted_networks.push(network);
    self
  }

  /// Checks a transport peer or a forwarded hop against the trust policy.
  pub fn is_trusted(&self, ip: &StdIpAddr) -> bool {
    self.trusted_proxies.contains(ip)
      || self
        .trusted_networks
        .iter()
        .any(|network| network.contains(ip))
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
      StdIpAddr::V4(ipv4) => ipv4.is_private() || ipv4.is_loopback(),
      StdIpAddr::V6(ipv6) => {
        ipv6.is_unique_local() || ipv6.is_unicast_link_local() || ipv6.is_loopback()
      }
    }
  }

  /// Resolves the client IP from request extensions + headers using the
  /// configured trust policy. Secure-by-default: forwarded headers are only
  /// honored when the direct peer is listed in `IpAddrConfig::trusted_proxies`.
  pub fn resolve(
    extensions: &http::Extensions,
    headers: &http::HeaderMap,
  ) -> Result<Self, IpAddrError> {
    let peer = peer_ip_from_extensions(extensions);

    let inherited = extensions
      .get::<Arc<tako_rs_core::router_state::RouterState>>()
      .and_then(|state| state.get::<IpAddrConfig>())
      .or_else(tako_rs_core::state::get_state::<IpAddrConfig>);
    let cfg = extensions.get::<IpAddrConfig>().or(inherited.as_deref());
    let trust_headers = match (peer.as_ref(), cfg.as_ref()) {
      (Some(p), Some(cfg)) => cfg.is_trusted(p),
      _ => false,
    };

    if trust_headers
      && let Some(cfg) = cfg.as_ref()
      && let Some(ip) = Self::parse_forwarded_headers(headers, cfg)
    {
      return Ok(Self(ip));
    }

    peer.map(Self).ok_or(IpAddrError::NoIpFound)
  }

  fn parse_forwarded_headers(
    headers: &http::HeaderMap,
    config: &IpAddrConfig,
  ) -> Option<StdIpAddr> {
    for name in ["forwarded", "x-forwarded-for"] {
      if headers.contains_key(name) {
        for value in headers.get_all(name).iter().rev() {
          for part in value.to_str().ok()?.rsplit(',') {
            // Opaque or malformed hops prevent trusting anything to their left.
            let ip = Self::parse_ip_from_part(part.trim())?;
            if !config.is_trusted(&ip) {
              return Some(ip);
            }
          }
        }
        return None;
      }
    }
    for name in [
      "x-real-ip",
      "x-client-ip",
      "cf-connecting-ip",
      "true-client-ip",
    ] {
      if let Some(value) = headers.get(name) {
        return Self::parse_ip_from_part(value.to_str().ok()?.trim());
      }
    }
    None
  }

  /// Parse one comma-separated entry into an IP, stripping `for=`, quotes,
  /// `[v6]` brackets, and an optional `:port` suffix.
  fn parse_ip_from_part(part: &str) -> Option<StdIpAddr> {
    let node = if part.contains('=') {
      let mut values = part.split(';').filter_map(|field| {
        let (name, value) = field.trim().split_once('=')?;
        name
          .trim()
          .eq_ignore_ascii_case("for")
          .then_some(value.trim())
      });
      let value = values.next()?;
      if values.next().is_some() {
        return None;
      }
      value
    } else {
      part
    };
    let node = if node.starts_with('"') || node.ends_with('"') {
      node.strip_prefix('"')?.strip_suffix('"')?
    } else {
      node
    };
    StdIpAddr::from_str(node)
      .ok()
      .or_else(|| SocketAddr::from_str(node).ok().map(|addr| addr.ip()))
      .or_else(|| node.strip_prefix('[')?.strip_suffix(']')?.parse().ok())
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
  const ENTRIES: Entries = Entries::CONN.union(Entries::STATE);

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::resolve(req.extensions(), req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for IpAddr {
  type Error = IpAddrError;
  const ENTRIES: Entries = Entries::CONN.union(Entries::STATE);

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::resolve(&parts.extensions, &parts.headers))
  }
}
