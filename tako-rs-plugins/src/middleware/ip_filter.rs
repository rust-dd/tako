//! CIDR allow and deny lists matched against the transport peer IP; deny wins.
//! `X-Forwarded-For` is ignored because clients control it.

use std::future::Future;
use std::net::IpAddr;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use ipnet::IpNet;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

#[derive(Clone)]
pub struct IpFilter {
  allow: Vec<IpNet>,
  deny: Vec<IpNet>,
  deny_unknown: bool,
  status: StatusCode,
}

impl Default for IpFilter {
  fn default() -> Self {
    Self::new()
  }
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

  /// Accepts a CIDR or a bare IP; a non-empty allow list rejects every IP outside it.
  pub fn allow(mut self, cidr: &str) -> Result<Self, ipnet::AddrParseError> {
    self.allow.push(parse_cidr(cidr)?);
    Ok(self)
  }

  /// Accepts a CIDR or a bare IP.
  pub fn deny(mut self, cidr: &str) -> Result<Self, ipnet::AddrParseError> {
    self.deny.push(parse_cidr(cidr)?);
    Ok(self)
  }

  /// Reject peers without an IP even with an empty allow list; defaults to false.
  /// A non-empty allow list always rejects peers without an IP.
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
  if let Ok(net) = cidr.parse::<IpNet>() {
    return Ok(net);
  }
  // `IpNet` needs a prefix, so a bare address becomes a /32 or /128 here.
  // `AddrParseError` has no public constructor.
  let ip: IpAddr = cidr
    .parse()
    .map_err(|_| "invalid".parse::<IpNet>().unwrap_err())?;
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
          None => deny_unknown || !allow.is_empty(),
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
