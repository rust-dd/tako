//! IP allow / deny filter with CIDR support.
//!
//! Reads the client IP from the unified [`ConnInfo`] extension (falling back
//! to the legacy `SocketAddr` extension), then matches it against an allow
//! list, a deny list, or both. Deny rules win when both match.
//!
//! `X-Forwarded-For` is intentionally not honored here — that path is the
//! responsibility of an upstream PROXY-protocol handler or an explicit
//! reverse-proxy gate. Trusting client-controlled headers in this layer
//! would let any caller spoof the source IP.

use std::future::Future;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use ipnet::IpNet;
use tako_core::body::TakoBody;
use tako_core::conn_info::ConnInfo;
use tako_core::conn_info::PeerAddr;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Allow / deny list of CIDR ranges.
#[derive(Default, Clone)]
pub struct IpFilter {
  allow: Vec<IpNet>,
  deny: Vec<IpNet>,
  /// When true, a request without a discoverable peer IP is denied. Default
  /// is false (allow when peer is unknown — typical for Unix sockets).
  deny_unknown: bool,
  /// Status returned on rejection.
  status: StatusCode,
}

impl IpFilter {
  /// Builds an empty filter (everything allowed).
  pub fn new() -> Self {
    Self {
      allow: Vec::new(),
      deny: Vec::new(),
      deny_unknown: false,
      status: StatusCode::FORBIDDEN,
    }
  }

  /// Adds a CIDR (or single IP) to the allow list.
  pub fn allow(mut self, cidr: &str) -> Result<Self, ipnet::AddrParseError> {
    self.allow.push(parse_cidr(cidr)?);
    Ok(self)
  }

  /// Adds a CIDR (or single IP) to the deny list.
  pub fn deny(mut self, cidr: &str) -> Result<Self, ipnet::AddrParseError> {
    self.deny.push(parse_cidr(cidr)?);
    Ok(self)
  }

  /// Reject requests whose peer IP cannot be determined.
  pub fn deny_unknown(mut self, deny: bool) -> Self {
    self.deny_unknown = deny;
    self
  }

  /// Override the default `403` rejection status.
  pub fn status(mut self, status: StatusCode) -> Self {
    self.status = status;
    self
  }
}

fn parse_cidr(cidr: &str) -> Result<IpNet, ipnet::AddrParseError> {
  // Single addresses (`1.2.3.4`) are parsed as `/32` or `/128` automatically.
  if let Ok(net) = cidr.parse::<IpNet>() {
    return Ok(net);
  }
  let ip: IpAddr = cidr.parse().map_err(|_| {
    // Re-parse to surface the original ipnet error rather than fabricating one.
    "invalid".parse::<IpNet>().unwrap_err()
  })?;
  Ok(IpNet::from(ip))
}

fn peer_ip(req: &Request) -> Option<IpAddr> {
  if let Some(info) = req.extensions().get::<ConnInfo>()
    && let PeerAddr::Ip(sa) = &info.peer
  {
    return Some(sa.ip());
  }
  if let Some(sa) = req.extensions().get::<SocketAddr>() {
    return Some(sa.ip());
  }
  None
}

impl IntoMiddleware for IpFilter {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let allow = Arc::new(self.allow);
    let deny = Arc::new(self.deny);
    let deny_unknown = self.deny_unknown;
    let status = self.status;

    move |req: Request, next: Next| {
      let allow = allow.clone();
      let deny = deny.clone();
      Box::pin(async move {
        let ip = peer_ip(&req);
        let reject = match ip {
          None => deny_unknown,
          Some(ip) => {
            if deny.iter().any(|n| n.contains(&ip)) {
              true
            } else if allow.is_empty() {
              false
            } else {
              !allow.iter().any(|n| n.contains(&ip))
            }
          }
        };
        if reject {
          return http::Response::builder()
            .status(status)
            .body(TakoBody::empty())
            .expect("valid ip_filter response");
        }
        next.run(req).await
      })
    }
  }
}
