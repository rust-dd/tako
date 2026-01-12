//! Client IP address extraction from HTTP request headers.
//!
//! This module provides the [`IpAddr`](crate::extractors::ipaddr::IpAddr) extractor for determining the client's IP address
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
use std::str::FromStr;

use http::StatusCode;
use http::request::Parts;

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::responder::Responder;
use crate::types::Request;

/// Extractor for client IP address from HTTP request headers.
///
/// This extractor attempts to determine the real client IP address by examining
/// various HTTP headers in priority order. It's particularly useful when your
/// application is behind proxies, load balancers, or CDNs that add forwarding headers.
///
/// The extractor checks headers in the following priority order:
/// 1. `X-Forwarded-For`
/// 2. `X-Real-IP`
/// 3. `X-Client-IP`
/// 4. `CF-Connecting-IP` (Cloudflare)
/// 5. `X-Forwarded`
/// 6. `Forwarded-For`
/// 7. `Forwarded`
/// 8. `True-Client-IP`
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
  fn into_response(self) -> crate::types::Response {
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
  /// Creates a new IpAddr wrapper.
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
  /// - fc00::/7 (Unique Local Addresses)
  /// - fe80::/10 (Link-Local Addresses)
  /// - ::1 (loopback)
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

  /// Extracts IP address from HTTP headers.
  fn extract_from_headers(headers: &http::HeaderMap) -> Result<Self, IpAddrError> {
    // Priority order of headers to check
    let header_names = [
      "x-forwarded-for",
      "x-real-ip",
      "x-client-ip",
      "cf-connecting-ip",
      "x-forwarded",
      "forwarded-for",
      "forwarded",
      "true-client-ip",
    ];

    for header_name in &header_names {
      if let Some(header_value) = headers.get(*header_name)
        && let Ok(header_str) = header_value.to_str()
        && let Some(ip) = Self::parse_ip_from_header(header_str)
      {
        return Ok(Self(ip));
      }
    }

    Err(IpAddrError::NoIpFound)
  }

  /// Parses an IP address from a header value.
  fn parse_ip_from_header(header_value: &str) -> Option<StdIpAddr> {
    // Handle comma-separated values (common in X-Forwarded-For)
    for part in header_value.split(',') {
      let part = part.trim();

      // Skip empty parts
      if part.is_empty() {
        continue;
      }

      // Handle "Forwarded" header format: for=192.168.1.1:1234
      let ip_part = if let Some(ip_part) = part.strip_prefix("for=") {
        ip_part
      } else {
        part
      };

      // Remove port if present (IPv4 format)
      let ip_str = if let Some(colon_pos) = ip_part.rfind(':') {
        // Check if this looks like IPv6 or IPv4:port
        if ip_part.starts_with('[') && ip_part.contains(']') {
          // IPv6 with port: [::1]:8080
          if let Some(bracket_end) = ip_part.find(']') {
            &ip_part[1..bracket_end]
          } else {
            ip_part
          }
        } else if ip_part.matches(':').count() == 1 {
          // IPv4 with port: 192.168.1.1:8080
          &ip_part[..colon_pos]
        } else {
          // IPv6 without brackets
          ip_part
        }
      } else {
        ip_part
      };

      // Try to parse as IP address
      if let Ok(ip) = StdIpAddr::from_str(ip_str) {
        // Skip local/private IPs in forwarded headers (optional filtering)
        // Comment out these lines if you want to accept private IPs
        match ip {
          StdIpAddr::V4(ipv4) if ipv4.is_loopback() || ipv4.is_private() => continue,
          StdIpAddr::V6(ipv6) if ipv6.is_loopback() => continue,
          _ => return Some(ip),
        }
      }
    }

    None
  }
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
    futures_util::future::ready(Self::extract_from_headers(req.headers()))
  }
}

impl<'a> FromRequestParts<'a> for IpAddr {
  type Error = IpAddrError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_headers(&parts.headers))
  }
}
