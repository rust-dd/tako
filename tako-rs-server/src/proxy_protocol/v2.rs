//! PROXY protocol v2 (binary format) parsing, including CRC32C verification.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
use std::net::SocketAddr;

use tokio::io::AsyncReadExt;

use super::header::MAX_PROXY_ADDR_LEN;
use super::header::PP2_TYPE_CRC32C;
use super::header::ProxyHeader;
use super::header::ProxyTransport;
use super::header::ProxyVersion;
use super::header::apply_tlvs;

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
/// Per the `HAProxy` PROXY v2 spec the checksum is computed over the entire
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
pub(crate) async fn parse_v2<R: AsyncReadExt + Unpin>(
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

  if addr_len > MAX_PROXY_ADDR_LEN {
    return Err(std::io::Error::new(
      std::io::ErrorKind::InvalidData,
      format!("PROXY v2 addr_len {addr_len} exceeds {MAX_PROXY_ADDR_LEN}"),
    ));
  }

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

/// Decode a NUL-terminated `AF_UNIX` path. Returns None if the path is empty.
fn parse_unix_path(bytes: &[u8]) -> Option<std::path::PathBuf> {
  let nul = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
  if nul == 0 {
    return None;
  }
  std::str::from_utf8(&bytes[..nul])
    .ok()
    .map(|s| std::path::PathBuf::from(s.to_string()))
}
