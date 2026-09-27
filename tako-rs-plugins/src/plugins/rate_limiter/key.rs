use std::net::IpAddr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;

use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::conn_info::PeerAddr;
use tako_rs_core::types::Request;

use super::config::Config;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum BucketKey {
  Ip(IpAddr),
  Custom(String),
}

pub(crate) fn default_key(request: &Request, config: &Config) -> Option<BucketKey> {
  let ip = if config.client_ip {
    tako_rs_extractors::ipaddr::IpAddr::resolve(request.extensions(), request.headers())
      .ok()?
      .0
  } else if let Some(info) = request.extensions().get::<ConnInfo>()
    && let PeerAddr::Ip(peer) = info.peer
  {
    peer.ip()
  } else {
    request.extensions().get::<SocketAddr>()?.ip()
  };
  let ip = match ip {
    IpAddr::V6(ip) => {
      if let Some(ip) = ip.to_ipv4_mapped() {
        IpAddr::V4(ip)
      } else {
        let mask = u128::MAX
          .checked_shl(128 - u32::from(config.ipv6_prefix))
          .unwrap_or(0);
        IpAddr::V6(Ipv6Addr::from(u128::from(ip) & mask))
      }
    }
    ip @ IpAddr::V4(_) => ip,
  };
  Some(BucketKey::Ip(ip))
}

impl BucketKey {
  pub(crate) fn storage_key(&self) -> std::borrow::Cow<'_, str> {
    match self {
      Self::Ip(ip) => format!("ip:{ip}").into(),
      Self::Custom(key) => key.as_str().into(),
    }
  }
}
