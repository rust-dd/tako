//! Unified connection-info extension shared by every Tako transport.
//!
//! Every `serve_*` implementation inserts a [`ConnInfo`] into the request's
//! extensions before dispatch, so handlers and middleware can read peer / TLS
//! metadata without branching on transport-specific extension types
//! (`SocketAddr`, `UnixPeerAddr`, `ProxyHeader`, …).
//!
//! Existing extension inserts (`SocketAddr`, `UnixPeerAddr`, `ProxyHeader`)
//! remain in place for backward compatibility — the new struct is additive.

use std::net::SocketAddr;
use std::path::PathBuf;

/// Network identity of a peer or local endpoint.
#[derive(Debug, Clone)]
pub enum PeerAddr {
  /// IPv4 / IPv6 socket address.
  Ip(SocketAddr),
  /// Unix domain socket path (None for unnamed client sockets).
  Unix(Option<PathBuf>),
  /// Reserved for vsock / abstract Unix / future transports.
  Other(String),
}

impl PeerAddr {
  /// Convenience: the `SocketAddr` if this is an [`PeerAddr::Ip`], else `None`.
  #[inline]
  pub fn as_socket(&self) -> Option<&SocketAddr> {
    match self {
      PeerAddr::Ip(addr) => Some(addr),
      _ => None,
    }
  }
}

impl From<SocketAddr> for PeerAddr {
  fn from(value: SocketAddr) -> Self {
    PeerAddr::Ip(value)
  }
}

/// Transport that produced the request. Lets handlers branch on plain HTTP/1
/// vs. TLS / HTTP/2 / HTTP/3 / Unix without keeping a parallel registry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
  /// Plain HTTP/1.1 over TCP.
  Http1,
  /// HTTP/2 (cleartext or via TLS).
  Http2,
  /// HTTP/3 over QUIC.
  Http3,
  /// HTTP/1.1 over Unix domain socket.
  Unix,
  /// Raw TCP — no HTTP wrapping (custom protocols only).
  Tcp,
}

/// TLS-specific connection metadata. Populated when the connection terminated
/// TLS at the server (TCP+TLS or HTTP/3); `None` for cleartext transports.
#[derive(Debug, Clone, Default)]
pub struct TlsInfo {
  /// Negotiated ALPN protocol (e.g. `b"h2"`, `b"http/1.1"`, `b"h3"`).
  pub alpn: Option<Vec<u8>>,
  /// SNI hostname presented by the client.
  pub sni: Option<String>,
  /// TLS protocol version label (e.g. `"TLSv1.3"`).
  pub version: Option<&'static str>,
}

/// Unified per-connection metadata, inserted into request extensions by every
/// transport before the router sees the request.
#[derive(Debug, Clone)]
pub struct ConnInfo {
  /// Remote (client) endpoint.
  pub peer: PeerAddr,
  /// Local endpoint, if known.
  pub local: Option<PeerAddr>,
  /// Transport identifier.
  pub transport: Transport,
  /// TLS metadata if the connection was terminated as TLS at this server.
  pub tls: Option<TlsInfo>,
}

impl ConnInfo {
  /// Helper for plain TCP HTTP/1 servers.
  #[inline]
  pub fn tcp(peer: SocketAddr) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      local: None,
      transport: Transport::Http1,
      tls: None,
    }
  }

  /// Helper for HTTP/2 over TLS connections.
  #[inline]
  pub fn h2_tls(peer: SocketAddr, tls: TlsInfo) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      local: None,
      transport: Transport::Http2,
      tls: Some(tls),
    }
  }

  /// Helper for plain HTTP/1 over TLS connections.
  #[inline]
  pub fn h1_tls(peer: SocketAddr, tls: TlsInfo) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      local: None,
      transport: Transport::Http1,
      tls: Some(tls),
    }
  }

  /// Helper for HTTP/3 connections.
  #[inline]
  pub fn h3(peer: SocketAddr, tls: TlsInfo) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      local: None,
      transport: Transport::Http3,
      tls: Some(tls),
    }
  }

  /// Helper for Unix domain socket connections.
  #[inline]
  pub fn unix(path: Option<PathBuf>) -> Self {
    Self {
      peer: PeerAddr::Unix(path),
      local: None,
      transport: Transport::Unix,
      tls: None,
    }
  }
}
