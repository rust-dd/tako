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

mod header;
mod listener;
mod v1;
mod v2;

pub use header::ProxyHeader;
pub use header::ProxyTlsInfo;
pub use header::ProxyTlv;
pub use header::ProxyTransport;
pub use header::ProxyVersion;
pub use listener::serve_http_with_proxy_protocol;
pub use listener::serve_http_with_proxy_protocol_and_config;
pub use listener::serve_http_with_proxy_protocol_and_shutdown;
pub use listener::serve_http_with_proxy_protocol_shutdown_and_config;
use tokio::io::AsyncReadExt;

use self::v1::parse_v1;
use self::v2::parse_v2;

/// PROXY protocol v2 binary signature (12 bytes).
const PROXY_V2_SIG: [u8; 12] = *b"\r\n\r\n\0\r\nQUIT\n";

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
