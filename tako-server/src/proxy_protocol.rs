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
use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;
use std::sync::Arc;

use hyper::server::conn::http1;
use hyper::service::service_fn;
use tako_core::body::TakoBody;
use tako_core::conn_info::ConnInfo;
use tako_core::router::Router;
use tako_core::types::BoxError;
use tokio::io::AsyncReadExt;
use tokio::task::JoinSet;

use crate::ServerConfig;

/// PROXY protocol v2 binary signature (12 bytes).
const PROXY_V2_SIG: [u8; 12] = *b"\r\n\r\n\0\r\nQUIT\n";

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

/// Raw PROXY v2 TLV (Type-Length-Value) field.
///
/// Most known TLV types (`authority`, `aws_vpc_endpoint_id`, `tls_*`) are also
/// surfaced as dedicated fields on [`ProxyHeader`]; `tlvs` keeps the unparsed
/// list so callers can extract custom or future-defined types.
#[derive(Debug, Clone)]
pub struct ProxyTlv {
  /// PP2_TYPE_* identifier byte.
  pub kind: u8,
  /// Raw TLV value bytes.
  pub value: Vec<u8>,
}

/// TLS-derived PROXY v2 sub-TLVs (PP2_TYPE_SSL container, type 0x20).
#[derive(Debug, Clone, Default)]
pub struct ProxyTlsInfo {
  /// PP2_CLIENT_SSL bitfield.
  pub client_flags: u8,
  /// rustls/openssl-style verify result code.
  pub verify: u32,
  /// PP2_SUBTYPE_SSL_VERSION (e.g. `"TLSv1.3"`).
  pub version: Option<String>,
  /// PP2_SUBTYPE_SSL_CN (peer common name).
  pub common_name: Option<String>,
  /// PP2_SUBTYPE_SSL_CIPHER.
  pub cipher: Option<String>,
  /// PP2_SUBTYPE_SSL_SIG_ALG.
  pub sig_alg: Option<String>,
  /// PP2_SUBTYPE_SSL_KEY_ALG.
  pub key_alg: Option<String>,
}

// PP2 TLV type identifiers (per HAProxy spec).
const PP2_TYPE_ALPN: u8 = 0x01;
const PP2_TYPE_AUTHORITY: u8 = 0x02;
const PP2_TYPE_CRC32C: u8 = 0x03;
const PP2_TYPE_NOOP: u8 = 0x04;
const PP2_TYPE_UNIQUE_ID: u8 = 0x05;
const PP2_TYPE_SSL: u8 = 0x20;
const PP2_SUBTYPE_SSL_VERSION: u8 = 0x21;
const PP2_SUBTYPE_SSL_CN: u8 = 0x22;
const PP2_SUBTYPE_SSL_CIPHER: u8 = 0x23;
const PP2_SUBTYPE_SSL_SIG_ALG: u8 = 0x24;
const PP2_SUBTYPE_SSL_KEY_ALG: u8 = 0x25;
const PP2_TYPE_NETNS: u8 = 0x30;
const PP2_TYPE_AWS_VPC_ENDPOINT_ID: u8 = 0xEA;

/// Parsed PROXY protocol header.
///
/// Contains the real client address (source) and the proxy/server address
/// (destination) extracted from the PROXY protocol header. PROXY v2 TLVs
/// (authority, AWS VPC endpoint ID, TLS info, …) are surfaced both as raw
/// [`ProxyTlv`]s and as typed fields where they map cleanly.
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
  /// AF_UNIX source path, when the connection family is Unix.
  pub source_unix: Option<std::path::PathBuf>,
  /// AF_UNIX destination path, when the connection family is Unix.
  pub destination_unix: Option<std::path::PathBuf>,
  /// PP2_TYPE_AUTHORITY (a.k.a. SNI) value if present.
  pub authority: Option<String>,
  /// PP2_TYPE_ALPN protocol bytes if present.
  pub alpn: Option<Vec<u8>>,
  /// AWS VPC endpoint identifier (PP2 type 0xEA) if present.
  pub aws_vpc_endpoint_id: Option<String>,
  /// Decoded PP2_TYPE_SSL sub-TLVs.
  pub tls: Option<ProxyTlsInfo>,
  /// Unique connection identifier (PP2 type 0x05).
  pub unique_id: Option<Vec<u8>>,
  /// Raw TLV list — kept for forward-compatibility / custom types.
  pub tlvs: Vec<ProxyTlv>,
  /// PROXY v2 CRC32C verification result.
  ///
  /// `Some(true)` if a `PP2_TYPE_CRC32C` TLV was present and the recomputed
  /// CRC32C of the full PROXY v2 header (with the CRC value zeroed) matched.
  /// `Some(false)` if it was present but mismatched. `None` if the TLV was
  /// absent (CRC32C is optional in the spec) or if the header is v1.
  pub crc32c_verified: Option<bool>,
}

impl ProxyHeader {
  fn empty(version: ProxyVersion, transport: ProxyTransport) -> Self {
    Self {
      version,
      transport,
      source: None,
      destination: None,
      source_unix: None,
      destination_unix: None,
      authority: None,
      alpn: None,
      aws_vpc_endpoint_id: None,
      tls: None,
      unique_id: None,
      tlvs: Vec::new(),
      crc32c_verified: None,
    }
  }
}

/// Walks a PROXY v2 TLV byte stream and applies each entry to a [`ProxyHeader`].
fn apply_tlvs(header: &mut ProxyHeader, mut buf: &[u8]) {
  while buf.len() >= 3 {
    let kind = buf[0];
    let len = u16::from_be_bytes([buf[1], buf[2]]) as usize;
    if buf.len() < 3 + len {
      break;
    }
    let value = &buf[3..3 + len];
    match kind {
      PP2_TYPE_ALPN => header.alpn = Some(value.to_vec()),
      PP2_TYPE_AUTHORITY => {
        if let Ok(s) = std::str::from_utf8(value) {
          header.authority = Some(s.to_string());
        }
      }
      PP2_TYPE_AWS_VPC_ENDPOINT_ID => {
        if let Ok(s) = std::str::from_utf8(value) {
          header.aws_vpc_endpoint_id = Some(s.to_string());
        }
      }
      PP2_TYPE_UNIQUE_ID => header.unique_id = Some(value.to_vec()),
      PP2_TYPE_SSL => {
        // PP2_TYPE_SSL container layout: 1 byte client flags, 4 bytes verify
        // (BE), then nested sub-TLVs.
        #[allow(clippy::collapsible_match)]
        if value.len() >= 5 {
          let mut tls = ProxyTlsInfo {
            client_flags: value[0],
            verify: u32::from_be_bytes([value[1], value[2], value[3], value[4]]),
            ..Default::default()
          };
          let mut sub = &value[5..];
          while sub.len() >= 3 {
            let sk = sub[0];
            let slen = u16::from_be_bytes([sub[1], sub[2]]) as usize;
            if sub.len() < 3 + slen {
              break;
            }
            let sval = &sub[3..3 + slen];
            match sk {
              PP2_SUBTYPE_SSL_VERSION => {
                tls.version = std::str::from_utf8(sval).ok().map(str::to_string)
              }
              PP2_SUBTYPE_SSL_CN => {
                tls.common_name = std::str::from_utf8(sval).ok().map(str::to_string)
              }
              PP2_SUBTYPE_SSL_CIPHER => {
                tls.cipher = std::str::from_utf8(sval).ok().map(str::to_string)
              }
              PP2_SUBTYPE_SSL_SIG_ALG => {
                tls.sig_alg = std::str::from_utf8(sval).ok().map(str::to_string)
              }
              PP2_SUBTYPE_SSL_KEY_ALG => {
                tls.key_alg = std::str::from_utf8(sval).ok().map(str::to_string)
              }
              _ => {}
            }
            sub = &sub[3 + slen..];
          }
          header.tls = Some(tls);
        }
      }
      // CRC32C is verified out-of-band by `verify_v2_crc32c` before TLV
      // expansion; NOOP / NETNS have no semantic meaning to surface.
      PP2_TYPE_CRC32C | PP2_TYPE_NOOP | PP2_TYPE_NETNS => {}
      _ => {}
    }
    header.tlvs.push(ProxyTlv {
      kind,
      value: value.to_vec(),
    });
    buf = &buf[3 + len..];
  }
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
  let text = std::str::from_utf8(&line).map_err(|_| {
    std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "invalid UTF-8 in PROXY v1 header",
    )
  })?;
  let text = text.trim_end_matches("\r\n");

  let parts: Vec<&str> = text.split(' ').collect();
  if parts.len() < 2 {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      "malformed PROXY v1 header",
    ));
  }

  match parts[1] {
    "UNKNOWN" => Ok(ProxyHeader::empty(
      ProxyVersion::V1,
      ProxyTransport::Unknown,
    )),
    proto @ ("TCP4" | "TCP6") => {
      if parts.len() < 6 {
        return Err(std::io::Error::new(
          std::io::ErrorKind::InvalidData,
          "incomplete PROXY v1 TCP header",
        ));
      }

      let src_ip: IpAddr = parts[2].parse().map_err(|e| {
        std::io::Error::new(
          std::io::ErrorKind::InvalidData,
          format!("bad source IP: {e}"),
        )
      })?;
      let dst_ip: IpAddr = parts[3].parse().map_err(|e| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bad dest IP: {e}"))
      })?;
      let src_port: u16 = parts[4].parse().map_err(|e| {
        std::io::Error::new(
          std::io::ErrorKind::InvalidData,
          format!("bad source port: {e}"),
        )
      })?;
      let dst_port: u16 = parts[5].parse().map_err(|e| {
        std::io::Error::new(
          std::io::ErrorKind::InvalidData,
          format!("bad dest port: {e}"),
        )
      })?;

      let transport = if proto.starts_with("TCP") {
        ProxyTransport::Tcp
      } else {
        ProxyTransport::Udp
      };

      let mut header = ProxyHeader::empty(ProxyVersion::V1, transport);
      header.source = Some(SocketAddr::new(src_ip, src_port));
      header.destination = Some(SocketAddr::new(dst_ip, dst_port));
      Ok(header)
    }
    other => Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      format!("unknown PROXY v1 protocol: {other}"),
    )),
  }
}

/// Walks the TLV section of a PROXY v2 header and returns the offset (relative
/// to the TLV section start) of the first `PP2_TYPE_CRC32C` value plus the
/// expected 32-bit checksum, if a well-formed CRC32C TLV is present.
fn locate_crc32c_tlv(mut buf: &[u8]) -> Option<(usize, u32)> {
  let mut offset = 0usize;
  while buf.len() >= 3 {
    let kind = buf[0];
    let len = u16::from_be_bytes([buf[1], buf[2]]) as usize;
    if buf.len() < 3 + len {
      return None;
    }
    if kind == PP2_TYPE_CRC32C && len == 4 {
      let value_offset = offset + 3;
      let v = &buf[3..7];
      let expected = u32::from_be_bytes([v[0], v[1], v[2], v[3]]);
      return Some((value_offset, expected));
    }
    buf = &buf[3 + len..];
    offset += 3 + len;
  }
  None
}

/// Verifies the PROXY v2 CRC32C TLV against the full reconstructed header.
///
/// Per the HAProxy PROXY v2 spec the checksum is computed over the entire
/// header (signature + version/command/family/protocol/length + addr + TLVs)
/// with the 4-byte CRC32C value field replaced by zeros. Returns `None` when
/// no CRC32C TLV is present.
fn verify_v2_crc32c(
  sig: &[u8; 12],
  hdr: &[u8; 4],
  addr_buf: &[u8],
  tlv_start: usize,
) -> Option<bool> {
  if tlv_start >= addr_buf.len() {
    return None;
  }
  let (value_offset_in_tlvs, expected) = locate_crc32c_tlv(&addr_buf[tlv_start..])?;
  let zero_at_in_addr = tlv_start + value_offset_in_tlvs;
  // Reconstruct the on-wire header into a single contiguous buffer.
  let mut full = Vec::with_capacity(12 + 4 + addr_buf.len());
  full.extend_from_slice(sig);
  full.extend_from_slice(hdr);
  full.extend_from_slice(addr_buf);
  let zero_at = 16 + zero_at_in_addr;
  full[zero_at..zero_at + 4].copy_from_slice(&[0, 0, 0, 0]);
  let computed = crc32c::crc32c(&full);
  Some(computed == expected)
}

/// Parse PROXY protocol v2 (binary format).
async fn parse_v2<R: AsyncReadExt + Unpin>(
  reader: &mut R,
  sig: &[u8; 12],
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
    return Ok(ProxyHeader::empty(
      ProxyVersion::V2,
      ProxyTransport::Unknown,
    ));
  }

  let transport = match protocol {
    1 => ProxyTransport::Tcp,
    2 => ProxyTransport::Udp,
    _ => ProxyTransport::Unknown,
  };

  let mut header = ProxyHeader::empty(ProxyVersion::V2, transport);

  let consumed: usize = match family {
    // AF_INET (IPv4)
    1 if addr_buf.len() >= 12 => {
      let src_ip = Ipv4Addr::new(addr_buf[0], addr_buf[1], addr_buf[2], addr_buf[3]);
      let dst_ip = Ipv4Addr::new(addr_buf[4], addr_buf[5], addr_buf[6], addr_buf[7]);
      let src_port = u16::from_be_bytes([addr_buf[8], addr_buf[9]]);
      let dst_port = u16::from_be_bytes([addr_buf[10], addr_buf[11]]);
      header.source = Some(SocketAddr::new(IpAddr::V4(src_ip), src_port));
      header.destination = Some(SocketAddr::new(IpAddr::V4(dst_ip), dst_port));
      12
    }
    // AF_INET6 (IPv6)
    2 if addr_buf.len() >= 36 => {
      let src_ip = Ipv6Addr::from(<[u8; 16]>::try_from(&addr_buf[0..16]).unwrap());
      let dst_ip = Ipv6Addr::from(<[u8; 16]>::try_from(&addr_buf[16..32]).unwrap());
      let src_port = u16::from_be_bytes([addr_buf[32], addr_buf[33]]);
      let dst_port = u16::from_be_bytes([addr_buf[34], addr_buf[35]]);
      header.source = Some(SocketAddr::new(IpAddr::V6(src_ip), src_port));
      header.destination = Some(SocketAddr::new(IpAddr::V6(dst_ip), dst_port));
      36
    }
    // AF_UNIX — 108-byte src + 108-byte dst NUL-terminated paths.
    3 if addr_buf.len() >= 216 => {
      let src = parse_unix_path(&addr_buf[0..108]);
      let dst = parse_unix_path(&addr_buf[108..216]);
      header.source_unix = src;
      header.destination_unix = dst;
      216
    }
    // UNSPEC or unknown — payload past addr_buf is still treated as TLVs.
    _ => 0,
  };

  // Verify the CRC32C TLV (if any) against the full reconstructed header
  // before TLV expansion so a corrupt payload does not silently mutate the
  // typed `ProxyHeader` fields. A mismatch is logged but does not abort the
  // parse — the operator decides via `ProxyHeader::crc32c_verified`.
  header.crc32c_verified = verify_v2_crc32c(sig, &hdr, &addr_buf, consumed);
  if header.crc32c_verified == Some(false) {
    tracing::warn!("PROXY v2 CRC32C mismatch — header may be corrupt or spoofed");
  }

  // Walk TLVs that follow the address payload.
  if consumed < addr_buf.len() {
    apply_tlvs(&mut header, &addr_buf[consumed..]);
  }

  Ok(header)
}

/// Decode a NUL-terminated AF_UNIX path. Returns None if the path is empty.
fn parse_unix_path(bytes: &[u8]) -> Option<std::path::PathBuf> {
  let nul = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
  if nul == 0 {
    return None;
  }
  std::str::from_utf8(&bytes[..nul])
    .ok()
    .map(|s| std::path::PathBuf::from(s.to_string()))
}

/// Starts an HTTP server that parses PROXY protocol headers on each connection.
///
/// The real client address from the PROXY header is inserted into request
/// extensions as `SocketAddr` (overriding the TCP peer address). The raw
/// `ProxyHeader` is also available via `req.extensions().get::<ProxyHeader>()`.
pub async fn serve_http_with_proxy_protocol(listener: tokio::net::TcpListener, router: Router) {
  if let Err(e) = run_proxy_http(
    listener,
    router,
    None::<std::future::Pending<()>>,
    ServerConfig::default(),
  )
  .await
  {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Starts an HTTP server with PROXY protocol support and graceful shutdown.
pub async fn serve_http_with_proxy_protocol_and_shutdown(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal), ServerConfig::default()).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Like [`serve_http_with_proxy_protocol`] with caller-supplied [`ServerConfig`].
pub async fn serve_http_with_proxy_protocol_and_config(
  listener: tokio::net::TcpListener,
  router: Router,
  config: ServerConfig,
) {
  if let Err(e) = run_proxy_http(listener, router, None::<std::future::Pending<()>>, config).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

/// Like [`serve_http_with_proxy_protocol_and_shutdown`] with caller-supplied [`ServerConfig`].
pub async fn serve_http_with_proxy_protocol_shutdown_and_config(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: impl Future<Output = ()>,
  config: ServerConfig,
) {
  if let Err(e) = run_proxy_http(listener, router, Some(signal), config).await {
    tracing::error!("PROXY protocol HTTP server error: {e}");
  }
}

async fn run_proxy_http(
  listener: tokio::net::TcpListener,
  router: Router,
  signal: Option<impl Future<Output = ()>>,
  config: ServerConfig,
) -> Result<(), BoxError> {
  let router = Arc::new(router);

  #[cfg(feature = "plugins")]
  router.setup_plugins_once();

  tracing::debug!(
    "Tako PROXY protocol HTTP listening on {}",
    listener.local_addr()?
  );

  let mut join_set = JoinSet::new();
  let mut accept_backoff = config.accept_backoff;
  let max_conn_semaphore = config
    .max_connections
    .map(|n| Arc::new(tokio::sync::Semaphore::new(n)));
  let drain_timeout = config.drain_timeout;
  let header_read_timeout = config.header_read_timeout;
  let keep_alive = config.keep_alive;
  let proxy_read_timeout = config.proxy_read_timeout;
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
        let (mut stream, _tcp_addr) = match result {
          Ok(v) => { accept_backoff.reset(); v }
          Err(err) => {
            tracing::warn!("PROXY accept failed: {err}; backing off");
            accept_backoff.sleep_and_grow().await;
            continue;
          }
        };
        let permit = if let Some(sem) = &max_conn_semaphore {
          match sem.clone().acquire_owned().await {
            Ok(p) => Some(p),
            Err(_) => continue,
          }
        } else {
          None
        };
        let _ = stream.set_nodelay(true);
        let router = router.clone();

        join_set.spawn(async move {
          // Parse PROXY protocol header under a read deadline so a stalled
          // client cannot pin a worker task forever.
          let proxy_header =
            match tokio::time::timeout(proxy_read_timeout, read_proxy_protocol(&mut stream)).await {
              Ok(Ok(h)) => h,
              Ok(Err(e)) => {
                tracing::warn!("Failed to parse PROXY protocol: {e}");
                return;
              }
              Err(_) => {
                tracing::warn!(
                  "PROXY protocol read deadline ({:?}) elapsed; dropping connection",
                  proxy_read_timeout,
                );
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
              // Strip any inbound X-Forwarded-For: clients behind a PROXY-protocol
              // hop must not be able to spoof their address through the header.
              // The PROXY-protocol-supplied source becomes the authoritative one.
              req.headers_mut().remove(http::header::FORWARDED);
              req.headers_mut().remove("x-forwarded-for");
              req.headers_mut().remove("x-forwarded-host");
              req.headers_mut().remove("x-forwarded-proto");

              if let Some(addr) = real_addr {
                req.extensions_mut().insert(addr);
                req.extensions_mut().insert(ConnInfo::tcp(addr));
              }
              req.extensions_mut().insert(proxy_header);
              let response = router.dispatch(req.map(TakoBody::incoming)).await;
              Ok::<_, Infallible>(response)
            }
          });

          let mut http = http1::Builder::new();
          http.keep_alive(keep_alive);
          http.timer(hyper_util::rt::TokioTimer::new());
          if let Some(t) = header_read_timeout {
            http.header_read_timeout(t);
          }
          let conn = http.serve_connection(io, svc).with_upgrades();

          if let Err(err) = conn.await {
            if err.is_incomplete_message() {
              tracing::debug!("client disconnected mid-message on PROXY protocol connection: {err}");
            } else {
              tracing::error!("Error serving PROXY protocol connection: {err}");
            }
          }

          drop(permit);
        });
      }
      () = &mut signal_fused => {
        tracing::info!("PROXY protocol HTTP server shutting down...");
        break;
      }
    }
  }

  let drain = tokio::time::timeout(drain_timeout, async {
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
