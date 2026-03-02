//! PROXY protocol v1/v2 parser for extracting real client addresses.
//!
//! When running behind load balancers (HAProxy, nginx, AWS ELB/NLB), the real
//! client IP is communicated via the PROXY protocol header prepended to the
//! TCP connection. This module parses both text (v1) and binary (v2) formats.
//!
//! # Examples
//!
//! ## With raw TCP server
//! ```rust,no_run
//! use tako::server_tcp::serve_tcp;
//! use tako::proxy_protocol::read_proxy_protocol;
//! use tokio::io::{AsyncReadExt, AsyncWriteExt};
//!
//! # async fn example() -> std::io::Result<()> {
//! serve_tcp("0.0.0.0:8080", |mut stream, _addr| {
//!     Box::pin(async move {
//!         let header = read_proxy_protocol(&mut stream).await?;
//!         println!("Real client: {:?}", header.source);
//!         // Continue reading HTTP or custom protocol data from stream...
//!         Ok(())
//!     })
//! }).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## HTTP server with PROXY protocol
//! ```rust,no_run
//! use tako::proxy_protocol::serve_http_with_proxy_protocol;
//! use tako::router::Router;
//!
//! # async fn example() {
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await.unwrap();
//! let router = Router::new();
//! serve_http_with_proxy_protocol(listener, router).await;
//! # }
//! ```

use std::convert::Infallible;
use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tokio::io::AsyncReadExt;
use tokio::task::JoinSet;

use crate::body::TakoBody;
use crate::router::Router;
use crate::types::BoxError;

/// PROXY protocol v2 binary signature (12 bytes).
const PROXY_V2_SIG: [u8; 12] = *b"\r\n\r\n\0\r\nQUIT\n";

/// Default drain timeout for graceful shutdown.
const DEFAULT_DRAIN_TIMEOUT: Duration = Duration::from_secs(30);

/// PROXY protocol version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyVersion {
  V1,
  V2,
}

/// Transport protocol from the PROXY header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyTransport {
  Tcp,
  Udp,
  Unknown,
}

/// Parsed PROXY protocol header.
///
/// Contains the real client address (source) and the proxy/server address
/// (destination) extracted from the PROXY protocol header.
#[derive(Debug, Clone)]
pub struct ProxyHeader {
  /// Protocol version (v1 text or v2 binary).
  pub version: ProxyVersion,
  /// Transport protocol.
  pub transport: ProxyTransport,
  /// Real client address (the original source).
  pub source: Option<SocketAddr>,
  /// Proxy/server address (the destination the client connected to).
  pub destination: Option<SocketAddr>,
}

/// Reads and parses a PROXY protocol header from a stream.
///
/// Supports both v1 (text) and v2 (binary) formats. After this function
/// returns, the stream is positioned right after the PROXY header and
/// ready for reading the actual protocol data (HTTP, etc.).
///
/// # Errors
///
/// Returns an error if the stream doesn't start with a valid PROXY protocol
/// header or if the header is malformed.
pub async fn read_proxy_protocol<R: AsyncReadExt + Unpin>(
  reader: &mut R,
) -> std::io::Result<ProxyHeader> {
  // Read first 12 bytes to determine version
  let mut sig = [0u8; 12];
  reader.read_exact(&mut sig).await?;

  if sig == PROXY_V2_SIG {
    parse_v2(reader, &sig).await
  } else if sig.starts_with(b"PROXY ") {
    parse_v1(reader, &sig).await
  } else {
    Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "invalid PROXY protocol header: unrecognized signature",
    ))
  }
}

/// Parse PROXY protocol v1 (text format).
///
/// Format: `PROXY TCP4|TCP6|UNKNOWN <src> <dst> <srcport> <dstport>\r\n`
async fn parse_v1<R: AsyncReadExt + Unpin>(
  reader: &mut R,
  initial: &[u8; 12],
) -> std::io::Result<ProxyHeader> {
  // We already have the first 12 bytes. Read until \r\n (max 107 bytes total).
  let mut line = Vec::from(&initial[..]);

  loop {
    let mut byte = [0u8; 1];
    reader.read_exact(&mut byte).await?;
    line.push(byte[0]);

    if line.ends_with(b"\r\n") {
      break;
    }
    if line.len() > 107 {
      return Err(std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "PROXY v1 header exceeds maximum length",
      ));
    }
  }

  // Parse: "PROXY TCP4 src dst srcport dstport\r\n"
  let text = std::str::from_utf8(&line)
    .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidData, "invalid UTF-8 in PROXY v1 header"))?;
  let text = text.trim_end_matches("\r\n");

  let parts: Vec<&str> = text.split(' ').collect();
  if parts.len() < 2 {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "malformed PROXY v1 header",
    ));
  }

  match parts[1] {
    "UNKNOWN" => Ok(ProxyHeader {
      version: ProxyVersion::V1,
      transport: ProxyTransport::Unknown,
      source: None,
      destination: None,
    }),
    proto @ ("TCP4" | "TCP6") => {
      if parts.len() < 6 {
        return Err(std::io::Error::new(
          std::io::ErrorKind::InvalidData,
          "incomplete PROXY v1 TCP header",
        ));
      }

      let src_ip: IpAddr = parts[2].parse().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad source IP: {e}"))
      })?;
      let dst_ip: IpAddr = parts[3].parse().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad dest IP: {e}"))
      })?;
      let src_port: u16 = parts[4].parse().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad source port: {e}"))
      })?;
      let dst_port: u16 = parts[5].parse().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad dest port: {e}"))
      })?;

      let transport = if proto.starts_with("TCP") {
        ProxyTransport::Tcp
      } else {
        ProxyTransport::Udp
      };

      Ok(ProxyHeader {
        version: ProxyVersion::V1,
        transport,
        source: Some(SocketAddr::new(src_ip, src_port)),
        destination: Some(SocketAddr::new(dst_ip, dst_port)),
      })
    }
    other => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      format!("unknown PROXY v1 protocol: {other}"),
    )),
  }
}

/// Parse PROXY protocol v2 (binary format).
async fn parse_v2<R: AsyncReadExt + Unpin>(
  reader: &mut R,
  _sig: &[u8; 12],
) -> std::io::Result<ProxyHeader> {
  // Read remaining 4 bytes of v2 header (version/command, family/protocol, length)
  let mut hdr = [0u8; 4];
  reader.read_exact(&mut hdr).await?;

  let ver_cmd = hdr[0];
  let version = (ver_cmd >> 4) & 0x0F;
  let command = ver_cmd & 0x0F;

  if version != 2 {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      format!("unsupported PROXY v2 version: {version}"),
    ));
  }

  let fam_proto = hdr[1];
  let family = (fam_proto >> 4) & 0x0F;
  let protocol = fam_proto & 0x0F;

  let addr_len = u16::from_be_bytes([hdr[2], hdr[3]]) as usize;

  // Read address data
  let mut addr_buf = vec![0u8; addr_len];
  if addr_len > 0 {
    reader.read_exact(&mut addr_buf).await?;
  }

  // LOCAL command: connection from proxy itself, no address info
  if command == 0 {
    return Ok(ProxyHeader {
      version: ProxyVersion::V2,
      transport: ProxyTransport::Unknown,
      source: None,
      destination: None,
    });
  }

  let transport = match protocol {
    1 => ProxyTransport::Tcp,
    2 => ProxyTransport::Udp,
    _ => ProxyTransport::Unknown,
  };

  match family {
    // AF_INET (IPv4)
    1 if addr_buf.len() >= 12 => {
      let src_ip = Ipv4Addr::new(addr_buf[0], addr_buf[1], addr_buf[2], addr_buf[3]);
      let dst_ip = Ipv4Addr::new(addr_buf[4], addr_buf[5], addr_buf[6], addr_buf[7]);
      let src_port = u16::from_be_bytes([addr_buf[8], addr_buf[9]]);
      let dst_port = u16::from_be_bytes([addr_buf[10], addr_buf[11]]);

      Ok(ProxyHeader {
        version: ProxyVersion::V2,
        transport,
        source: Some(SocketAddr::new(IpAddr::V4(src_ip), src_port)),
        destination: Some(SocketAddr::new(IpAddr::V4(dst_ip), dst_port)),
      })
    }
    // AF_INET6 (IPv6)
    2 if addr_buf.len() >= 36 => {
      let src_ip = Ipv6Addr::from(<[u8; 16]>::try_from(&addr_buf[0..16]).unwrap());
      let dst_ip = Ipv6Addr::from(<[u8; 16]>::try_from(&addr_buf[16..32]).unwrap());
      let src_port = u16::from_be_bytes([addr_buf[32], addr_buf[33]]);
      let dst_port = u16::from_be_bytes([addr_buf[34], addr_buf[35]]);

      Ok(ProxyHeader {
        version: ProxyVersion::V2,
        transport,
        source: Some(SocketAddr::new(IpAddr::V6(src_ip), src_port)),
        destination: Some(SocketAddr::new(IpAddr::V6(dst_ip), dst_port)),
      })
    }
    // UNSPEC or unknown
    _ => Ok(ProxyHeader {
      version: ProxyVersion::V2,
      transport,
      source: None,
      destination: None,
    }),
  }
}

// ─── HTTP server with PROXY protocol ───

/// Starts an HTTP server that parses PROXY protocol headers on each connection.
///
/// The real client address from the PROXY header is inserted into request
/// extensions as `SocketAddr` (overriding the TCP peer address). The raw
/// `ProxyHeader` is also available via `req.extensions().get::<ProxyHeader>()`.
pub async fn serve_http_with_proxy_protocol(listener: tokio::net::TcpListener, router: Router) {
  if let Err(e) = run_proxy_http(listener, router, None::<std::future::Pending<()>>).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Starts an HTTP server with PROXY protocol support and graceful shutdown.
pub async fn serve_http_with_proxy_protocol_and_shutdown(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal)).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

async fn run_proxy_http(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
) -> Result<(), BoxError> {
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  tracing::debug!(
    "Tako PROXY protocol HTTP listening on {}",
    listener.local_addr()?
  );

  let mut join_set = JoinSet::new();
  let signal = signal.map(|s| Box::pin(s));
  let signal_fused = async {
    if let Some(s) = signal {
      s.await;
    } else {
      std::future::pending::<()>().await;
    }
  };
  tokio::pin!(signal_fused);

  loop {
    tokio::select! {
      result = listener.accept() => {
        let (mut stream, _tcp_addr) = result?;
        let router = router.clone();

        join_set.spawn(async move {
          // Parse PROXY protocol header
          let proxy_header = match read_proxy_protocol(&mut stream).await {
            Ok(h) => h,
            Err(e) => {
              tracing::error!("Failed to parse PROXY protocol: {e}");
              return;
            }
          };

          let real_addr = proxy_header.source;
          let io = hyper_util::rt::TokioIo::new(stream);

          let svc = service_fn(move |mut req| {
            let router = router.clone();
            let proxy_header = proxy_header.clone();
            let real_addr = real_addr;
            async move {
              // Insert real client address
              if let Some(addr) = real_addr {
                req.extensions_mut().insert(addr);
              }
              req.extensions_mut().insert(proxy_header);
              let response = router.dispatch(req.map(TakoBody::new)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(true);
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            tracing::error!("Error serving PROXY protocol connection: {err}");
          }
        });
      }
      () = &mut signal_fused => {
        tracing::info!("PROXY protocol HTTP server shutting down...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(DEFAULT_DRAIN_TIMEOUT, async {
    while join_set.join_next().await.is_some() {}
  });

  if drain.await.is_err() {
    tracing::warn!(
      "Drain timeout exceeded, aborting {} remaining connections",
      join_set.len()
    );
    join_set.abort_all();
  }

  tracing::info!("PROXY protocol HTTP server shut down gracefully");
  Ok(())
}
