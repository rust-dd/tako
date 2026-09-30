//! PROXY protocol v1/v2 parser for extracting real client addresses.
//!
//! When running behind load balancers (`HAProxy`, nginx, AWS ELB/NLB), the real
//! client IP is communicated via the PROXY protocol header prepended to the
//! TCP connection. This module parses both text (v1) and binary (v2) formats.
//!
//! # Examples
//!
//! ## With raw TCP server
//! ```rust,no_run
//! use tako::server_tcp::serve_tcp;
//! use tako::proxy_protocol::read_proxy_protocol;
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
//! use tako::router::Router;
//!
//! # #[cfg(not(feature = "compio"))]
//! # async fn example() -> Result<(), tako::types::BoxError> {
//! let listener = tokio::net::TcpListener::bind("0.0.0.0:8080").await?;
//! let handle = tako::Server::builder()
//!     .build()
//!     .try_spawn_proxy_protocol(listener, Router::new())?;
//! handle.result().await?;
//! # Ok(())
//! # }
//! # #[cfg(feature = "compio")]
//! # async fn example() -> Result<(), tako::types::BoxError> {
//! # let listener = compio::net::TcpListener::bind("0.0.0.0:8080").await?;
//! # let handle = tako::CompioServer::builder()
//! #     .build()
//! #     .try_spawn_proxy_protocol(listener, Router::new())?;
//! # handle.result().await?;
//! # Ok(())
//! # }
//! ```

use std::net::SocketAddr;

use tako_rs_core::conn_info::ConnInfo;

mod header;
#[cfg(not(feature = "compio"))]
pub(crate) mod listener;
mod v1;
mod v2;

pub use header::ProxyHeader;
pub use header::ProxyTlsInfo;
pub use header::ProxyTlv;
pub use header::ProxyTransport;
pub use header::ProxyVersion;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use listener::serve_http_with_proxy_protocol;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use listener::serve_http_with_proxy_protocol_and_config;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use listener::serve_http_with_proxy_protocol_and_shutdown;
#[cfg(not(feature = "compio"))]
#[allow(deprecated)]
pub use listener::serve_http_with_proxy_protocol_shutdown_and_config;

/// PROXY protocol v2 binary signature (12 bytes).
const PROXY_V2_SIG: [u8; 12] = *b"\r\n\r\n\0\r\nQUIT\n";

/// Header version announced by the first 12 bytes on the wire.
enum Signature {
  V1,
  V2,
}

fn signature(sig: &[u8; 12]) -> std::io::Result<Signature> {
  if *sig == PROXY_V2_SIG {
    Ok(Signature::V2)
  } else if sig.starts_with(b"PROXY ") {
    Ok(Signature::V1)
  } else {
    Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "invalid PROXY protocol header: unrecognized signature",
    ))
  }
}

/// Reads and parses a PROXY protocol header from a stream (tokio runtime).
///
/// Supports both v1 (text) and v2 (binary) formats. After this function
/// returns, the stream is positioned right after the PROXY header and
/// ready for reading the actual protocol data (HTTP, etc.).
///
/// # Errors
///
/// Returns an error if the stream doesn't start with a valid PROXY protocol
/// header or if the header is malformed.
#[cfg(not(feature = "compio"))]
pub async fn read_proxy_protocol<R: tokio::io::AsyncReadExt + Unpin>(
  reader: &mut R,
) -> std::io::Result<ProxyHeader> {
  let mut sig = [0u8; 12];
  reader.read_exact(&mut sig).await?;

  match signature(&sig)? {
    Signature::V2 => {
      let mut hdr = [0u8; 4];
      reader.read_exact(&mut hdr).await?;
      let mut addr_buf = vec![0u8; v2::address_len(&hdr)?];
      if !addr_buf.is_empty() {
        reader.read_exact(&mut addr_buf).await?;
      }
      Ok(v2::parse_v2(&sig, &hdr, &addr_buf))
    }
    Signature::V1 => {
      // Byte by byte, so nothing past the CRLF is consumed from the stream.
      let mut line = Vec::from(&sig[..]);
      loop {
        let mut byte = [0u8; 1];
        reader.read_exact(&mut byte).await?;
        line.push(byte[0]);
        if v1::line_complete(&line)? {
          break;
        }
      }
      v1::parse_v1(&line)
    }
  }
}

/// Reads and parses a PROXY protocol header from a stream (compio runtime).
///
/// Supports both v1 (text) and v2 (binary) formats. After this function
/// returns, the stream is positioned right after the PROXY header and
/// ready for reading the actual protocol data (HTTP, etc.).
///
/// # Errors
///
/// Returns an error if the stream doesn't start with a valid PROXY protocol
/// header or if the header is malformed.
#[cfg(feature = "compio")]
pub async fn read_proxy_protocol<R: compio::io::AsyncRead>(
  reader: &mut R,
) -> std::io::Result<ProxyHeader> {
  let sig: [u8; 12] = read_exact_compio(reader, vec![0u8; 12])
    .await?
    .try_into()
    .expect("read_exact fills the 12-byte signature");

  match signature(&sig)? {
    Signature::V2 => {
      let hdr: [u8; 4] = read_exact_compio(reader, vec![0u8; 4])
        .await?
        .try_into()
        .expect("read_exact fills the 4-byte header");
      let addr_len = v2::address_len(&hdr)?;
      let addr_buf = if addr_len == 0 {
        Vec::new()
      } else {
        read_exact_compio(reader, vec![0u8; addr_len]).await?
      };
      Ok(v2::parse_v2(&sig, &hdr, &addr_buf))
    }
    Signature::V1 => {
      // Byte by byte, so nothing past the CRLF is consumed from the stream.
      let mut line = Vec::from(&sig[..]);
      let mut byte = vec![0u8; 1];
      loop {
        byte = read_exact_compio(reader, byte).await?;
        line.push(byte[0]);
        if v1::line_complete(&line)? {
          break;
        }
      }
      v1::parse_v1(&line)
    }
  }
}

#[cfg(feature = "compio")]
async fn read_exact_compio<R: compio::io::AsyncRead>(
  reader: &mut R,
  buf: Vec<u8>,
) -> std::io::Result<Vec<u8>> {
  let compio::BufResult(result, buf) = compio::io::AsyncReadExt::read_exact(reader, buf).await;
  result.map(|()| buf)
}

/// Replaces client-supplied forwarding headers with one `Forwarded` entry
/// for the PROXY source and exposes that source as the request's peer.
///
/// Stripping `Forwarded` and `X-Forwarded-*` stops a client behind the load
/// balancer from spoofing its address; the header was already vouched for by
/// the PROXY hop.
pub(crate) fn apply_to_request<B>(req: &mut http::Request<B>, header: &ProxyHeader) {
  let headers = req.headers_mut();
  headers.remove(http::header::FORWARDED);
  headers.remove("x-forwarded-for");
  headers.remove("x-forwarded-host");
  headers.remove("x-forwarded-proto");

  if let Some(addr) = header.source {
    if let Ok(value) = http::HeaderValue::from_str(&forwarded_for(addr)) {
      headers.insert(http::header::FORWARDED, value);
    }
    req.extensions_mut().insert(addr);
    req.extensions_mut().insert(ConnInfo::tcp(addr));
  }
  req.extensions_mut().insert(header.clone());
}

fn forwarded_for(addr: SocketAddr) -> String {
  match addr {
    SocketAddr::V4(v4) => format!("for=\"{}:{}\"", v4.ip(), v4.port()),
    SocketAddr::V6(v6) => format!("for=\"[{}]:{}\"", v6.ip(), v6.port()),
  }
}
