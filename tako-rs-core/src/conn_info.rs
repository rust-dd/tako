//! [`ConnInfo`], which every transport adds to request extensions next to its own entries,
//! such as `SocketAddr` or [`UnixPeerAddr`].

use std::net::SocketAddr;
use std::path::PathBuf;

/// Network identity of a peer or local endpoint.
#[derive(Debug, Clone)]
pub enum PeerAddr {
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

/// The protocol a request arrived over; TLS is reported separately in [`ConnInfo::tls`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
  /// HTTP/1.x over TCP, with or without TLS.
  Http1,
  /// HTTP/2 (cleartext or via TLS).
  Http2,
  /// HTTP/3 over QUIC.
  Http3,
  /// HTTP/1.1 over Unix domain socket.
  Unix,
}

/// TLS-specific connection metadata. Populated when the connection terminated
/// TLS at the server (TCP+TLS or HTTP/3); `None` for cleartext transports.
#[derive(Debug, Clone, Default)]
pub struct TlsInfo {
  /// Negotiated ALPN protocol (e.g. `b"h2"`, `b"http/1.1"`, `b"h3"`).
  pub alpn: Option<bytes::Bytes>,
  /// SNI hostname presented by the client.
  pub sni: Option<std::sync::Arc<str>>,
  /// TLS protocol version label (e.g. `"TLSv1.3"`).
  pub version: Option<&'static str>,
}

/// Unified per-connection metadata, inserted into request extensions by every
/// transport before the router sees the request.
#[derive(Debug, Clone)]
pub struct ConnInfo {
  /// Remote (client) endpoint.
  pub peer: PeerAddr,
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
      transport: Transport::Http1,
      tls: None,
    }
  }

  /// Helper for HTTP/2 over TLS connections.
  #[inline]
  pub fn h2_tls(peer: SocketAddr, tls: TlsInfo) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      transport: Transport::Http2,
      tls: Some(tls),
    }
  }

  /// Helper for cleartext HTTP/2 (h2c) connections.
  #[inline]
  pub fn h2c(peer: SocketAddr) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      transport: Transport::Http2,
      tls: None,
    }
  }

  /// Helper for plain HTTP/1 over TLS connections.
  #[inline]
  pub fn h1_tls(peer: SocketAddr, tls: TlsInfo) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      transport: Transport::Http1,
      tls: Some(tls),
    }
  }

  /// Helper for HTTP/3 connections.
  #[inline]
  pub fn h3(peer: SocketAddr, tls: TlsInfo) -> Self {
    Self {
      peer: PeerAddr::Ip(peer),
      transport: Transport::Http3,
      tls: Some(tls),
    }
  }

  /// Helper for Unix domain socket connections.
  #[inline]
  pub fn unix(path: Option<PathBuf>) -> Self {
    Self {
      peer: PeerAddr::Unix(path),
      transport: Transport::Unix,
      tls: None,
    }
  }
}

/// Peer address of an HTTP-over-Unix-socket connection, inserted into request
/// extensions next to [`ConnInfo`] on both runtimes.
#[derive(Debug, Clone)]
pub struct UnixPeerAddr {
  /// The filesystem path of the peer socket, if available.
  /// Most client connections are unnamed (None).
  pub path: Option<PathBuf>,
}
