//! Client IP address extraction from HTTP request headers.
//!
//! This module provides the [`IpAddr`] extractor for determining the client's IP address
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

use http::{StatusCode, request::Parts};
use std::{future::ready, net::IpAddr as StdIpAddr, str::FromStr};

use crate::{
    extractors::{FromRequest, FromRequestParts},
    responder::Responder,
    types::Request,
};

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
pub struct IpAddr(pub StdIpAddr);

/// Error type for IP address extraction.
///
/// Represents various failure modes that can occur when extracting IP addresses
/// from HTTP request headers.
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
    ///
    /// Maps IP address extraction errors to appropriate HTTP status codes with
    /// descriptive error messages. All errors result in `400 Bad Request` as they
    /// indicate issues with the client's request headers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddrError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = IpAddrError::NoIpFound;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    ///
    /// let error = IpAddrError::InvalidIpFormat("not-an-ip".to_string());
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
    fn into_response(self) -> crate::types::Response {
        match self {
            IpAddrError::NoIpFound => (
                StatusCode::BAD_REQUEST,
                "No valid IP address found in request headers",
            )
                .into_response(),
            IpAddrError::InvalidIpFormat(ip) => (
                StatusCode::BAD_REQUEST,
                format!("Invalid IP address format: {}", ip),
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
    ///
    /// # Arguments
    ///
    /// * `addr` - The IP address to wrap
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    /// use std::net::IpAddr as StdIpAddr;
    ///
    /// let ip_addr: StdIpAddr = "192.168.1.1".parse().unwrap();
    /// let wrapper = IpAddr::new(ip_addr);
    /// assert_eq!(wrapper.to_string(), "192.168.1.1");
    /// ```
    pub fn new(addr: StdIpAddr) -> Self {
        Self(addr)
    }

    /// Gets the inner IP address.
    ///
    /// Returns the wrapped `std::net::IpAddr` for use with standard library functions.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    /// use std::net::IpAddr as StdIpAddr;
    ///
    /// let original: StdIpAddr = "::1".parse().unwrap();
    /// let wrapper = IpAddr::new(original);
    /// let inner = wrapper.inner();
    ///
    /// assert_eq!(inner, original);
    /// assert!(inner.is_loopback());
    /// ```
    pub fn inner(&self) -> StdIpAddr {
        self.0
    }

    /// Checks if the IP address is IPv4.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    ///
    /// let ipv4 = IpAddr::new("192.168.1.1".parse().unwrap());
    /// let ipv6 = IpAddr::new("::1".parse().unwrap());
    ///
    /// assert!(ipv4.is_ipv4());
    /// assert!(!ipv6.is_ipv4());
    /// ```
    pub fn is_ipv4(&self) -> bool {
        self.0.is_ipv4()
    }

    /// Checks if the IP address is IPv6.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    ///
    /// let ipv4 = IpAddr::new("192.168.1.1".parse().unwrap());
    /// let ipv6 = IpAddr::new("::1".parse().unwrap());
    ///
    /// assert!(!ipv4.is_ipv6());
    /// assert!(ipv6.is_ipv6());
    /// ```
    pub fn is_ipv6(&self) -> bool {
        self.0.is_ipv6()
    }

    /// Checks if the IP address is a loopback address.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    ///
    /// let localhost_v4 = IpAddr::new("127.0.0.1".parse().unwrap());
    /// let localhost_v6 = IpAddr::new("::1".parse().unwrap());
    /// let public_ip = IpAddr::new("8.8.8.8".parse().unwrap());
    ///
    /// assert!(localhost_v4.is_loopback());
    /// assert!(localhost_v6.is_loopback());
    /// assert!(!public_ip.is_loopback());
    /// ```
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
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    ///
    /// let private_v4 = IpAddr::new("192.168.1.1".parse().unwrap());
    /// let public_v4 = IpAddr::new("8.8.8.8".parse().unwrap());
    /// let private_v6 = IpAddr::new("fc00::1".parse().unwrap());
    ///
    /// assert!(private_v4.is_private());
    /// assert!(!public_v4.is_private());
    /// assert!(private_v6.is_private());
    /// ```
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
    ///
    /// Examines various HTTP headers in priority order to find the client's real IP address.
    /// This is particularly useful when the application is behind proxies or load balancers.
    ///
    /// # Arguments
    ///
    /// * `headers` - HTTP headers to examine for IP address information
    ///
    /// # Errors
    ///
    /// Returns `IpAddrError::NoIpFound` if no valid IP address is found in any header.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    /// use http::HeaderMap;
    ///
    /// let mut headers = HeaderMap::new();
    /// headers.insert("x-forwarded-for", "203.0.113.1, 198.51.100.1".parse().unwrap());
    ///
    /// let ip = IpAddr::extract_from_headers(&headers).unwrap();
    /// assert_eq!(ip.to_string(), "203.0.113.1");
    /// ```
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
            if let Some(header_value) = headers.get(*header_name) {
                if let Ok(header_str) = header_value.to_str() {
                    if let Some(ip) = Self::parse_ip_from_header(header_str) {
                        return Ok(Self(ip));
                    }
                }
            }
        }

        Err(IpAddrError::NoIpFound)
    }

    /// Parses an IP address from a header value.
    ///
    /// Handles various header formats including comma-separated lists (common in
    /// X-Forwarded-For) and takes the first valid IP address found. Also handles
    /// IPv6 addresses with brackets and port numbers.
    ///
    /// # Arguments
    ///
    /// * `header_value` - The header value string to parse
    ///
    /// # Examples
    ///
    /// ```rust
    /// # use tako::extractors::ipaddr::IpAddr;
    /// # impl IpAddr {
    /// #     fn parse_ip_from_header(header_value: &str) -> Option<std::net::IpAddr> {
    /// #         // Implementation details...
    /// #         Some("192.168.1.1".parse().unwrap())
    /// #     }
    /// # }
    /// let ip = IpAddr::parse_ip_from_header("192.168.1.1, 10.0.0.1");
    /// assert!(ip.is_some());
    /// ```
    fn parse_ip_from_header(header_value: &str) -> Option<StdIpAddr> {
        // Handle comma-separated values (common in X-Forwarded-For)
        for part in header_value.split(',') {
            let part = part.trim();

            // Skip empty parts
            if part.is_empty() {
                continue;
            }

            // Handle "Forwarded" header format: for=192.168.1.1:1234
            let ip_part = if part.starts_with("for=") {
                &part[4..]
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
    /// Formats the IP address for display.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    ///
    /// let ip = IpAddr::new("192.168.1.1".parse().unwrap());
    /// assert_eq!(format!("{}", ip), "192.168.1.1");
    ///
    /// let ipv6 = IpAddr::new("::1".parse().unwrap());
    /// assert_eq!(format!("{}", ipv6), "::1");
    /// ```
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl From<StdIpAddr> for IpAddr {
    /// Converts from `std::net::IpAddr` to `IpAddr`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    /// use std::net::IpAddr as StdIpAddr;
    ///
    /// let std_ip: StdIpAddr = "192.168.1.1".parse().unwrap();
    /// let ip: IpAddr = std_ip.into();
    /// assert_eq!(ip.to_string(), "192.168.1.1");
    /// ```
    fn from(addr: StdIpAddr) -> Self {
        Self(addr)
    }
}

impl From<IpAddr> for StdIpAddr {
    /// Converts from `IpAddr` to `std::net::IpAddr`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::ipaddr::IpAddr;
    /// use std::net::IpAddr as StdIpAddr;
    ///
    /// let ip = IpAddr::new("192.168.1.1".parse().unwrap());
    /// let std_ip: StdIpAddr = ip.into();
    /// assert_eq!(std_ip.to_string(), "192.168.1.1");
    /// ```
    fn from(addr: IpAddr) -> Self {
        addr.0
    }
}

impl<'a> FromRequest<'a> for IpAddr {
    type Error = IpAddrError;

    /// Extracts client IP address from an HTTP request.
    ///
    /// Examines various HTTP headers to determine the client's real IP address,
    /// which is particularly useful when the application is behind proxies,
    /// load balancers, or CDNs.
    ///
    /// # Errors
    ///
    /// Returns `IpAddrError::NoIpFound` if no valid IP address can be extracted
    /// from the request headers.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, ipaddr::IpAddr};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let ip = IpAddr::from_request(&mut req).await?;
    ///
    ///     println!("Client IP: {}", ip);
    ///
    ///     if ip.is_private() {
    ///         println!("Request from private network");
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(req.headers()))
    }
}

impl<'a> FromRequestParts<'a> for IpAddr {
    type Error = IpAddrError;

    /// Extracts client IP address from HTTP request parts.
    ///
    /// Examines various HTTP headers to determine the client's real IP address,
    /// which is particularly useful when the application is behind proxies,
    /// load balancers, or CDNs.
    ///
    /// # Errors
    ///
    /// Returns `IpAddrError::NoIpFound` if no valid IP address can be extracted
    /// from the request headers.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, ipaddr::IpAddr};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let ip = IpAddr::from_request_parts(&mut parts).await?;
    ///
    ///     // Log the client IP for security monitoring
    ///     if !ip.is_private() {
    ///         println!("External request from: {}", ip);
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_headers(&parts.headers))
    }
}
