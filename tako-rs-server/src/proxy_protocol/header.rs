//! PROXY protocol address/header value types and TLV expansion.

use std::net::SocketAddr;

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
  /// `PP2_TYPE_*` identifier byte.
  pub kind: u8,
  /// Raw TLV value bytes.
  pub value: Vec<u8>,
}

/// TLS-derived PROXY v2 sub-TLVs (`PP2_TYPE_SSL` container, type 0x20).
#[derive(Debug, Clone, Default)]
pub struct ProxyTlsInfo {
  /// `PP2_CLIENT_SSL` bitfield.
  pub client_flags: u8,
  /// rustls/openssl-style verify result code.
  pub verify: u32,
  /// `PP2_SUBTYPE_SSL_VERSION` (e.g. `"TLSv1.3"`).
  pub version: Option<String>,
  /// `PP2_SUBTYPE_SSL_CN` (peer common name).
  pub common_name: Option<String>,
  /// `PP2_SUBTYPE_SSL_CIPHER`.
  pub cipher: Option<String>,
  /// `PP2_SUBTYPE_SSL_SIG_ALG`.
  pub sig_alg: Option<String>,
  /// `PP2_SUBTYPE_SSL_KEY_ALG`.
  pub key_alg: Option<String>,
}

// Cap the advertised address length so an attacker can't pre-allocate
// 64 KiB per connection just by sending two unfavourable bytes. The
// PROXY v2 spec's typed payload for IPv6+UNIX maxes out far below this:
// IPv4(12) + IPv6(36) + UNIX(216) + a reasonable TLV stack (~256). 536
// bytes is generous; legitimate proxies should never exceed it.
pub(crate) const MAX_PROXY_ADDR_LEN: usize = 536;

// PP2 TLV type identifiers (per HAProxy spec).
const PP2_TYPE_ALPN: u8 = 0x01;
const PP2_TYPE_AUTHORITY: u8 = 0x02;
pub(crate) const PP2_TYPE_CRC32C: u8 = 0x03;
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
  /// `AF_UNIX` source path, when the connection family is Unix.
  pub source_unix: Option<std::path::PathBuf>,
  /// `AF_UNIX` destination path, when the connection family is Unix.
  pub destination_unix: Option<std::path::PathBuf>,
  /// `PP2_TYPE_AUTHORITY` (a.k.a. SNI) value if present.
  pub authority: Option<String>,
  /// `PP2_TYPE_ALPN` protocol bytes if present.
  pub alpn: Option<Vec<u8>>,
  /// AWS VPC endpoint identifier (`PP2` type 0xEA) if present.
  pub aws_vpc_endpoint_id: Option<String>,
  /// Decoded `PP2_TYPE_SSL` sub-TLVs.
  pub tls: Option<ProxyTlsInfo>,
  /// Unique connection identifier (`PP2` type 0x05).
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
  pub(crate) fn empty(version: ProxyVersion, transport: ProxyTransport) -> Self {
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
pub(crate) fn apply_tlvs(header: &mut ProxyHeader, mut buf: &[u8]) {
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
                tls.version = std::str::from_utf8(sval).ok().map(str::to_string);
              }
              PP2_SUBTYPE_SSL_CN => {
                tls.common_name = std::str::from_utf8(sval).ok().map(str::to_string);
              }
              PP2_SUBTYPE_SSL_CIPHER => {
                tls.cipher = std::str::from_utf8(sval).ok().map(str::to_string);
              }
              PP2_SUBTYPE_SSL_SIG_ALG => {
                tls.sig_alg = std::str::from_utf8(sval).ok().map(str::to_string);
              }
              PP2_SUBTYPE_SSL_KEY_ALG => {
                tls.key_alg = std::str::from_utf8(sval).ok().map(str::to_string);
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
