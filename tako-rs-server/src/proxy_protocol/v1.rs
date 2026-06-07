//! PROXY protocol v1 (text format) parsing.

use std::net::IpAddr;
use std::net::SocketAddr;

use tokio::io::AsyncReadExt;

use super::header::ProxyHeader;
use super::header::ProxyTransport;
use super::header::ProxyVersion;

/// Parse PROXY protocol v1 (text format).
///
/// Format: `PROXY TCP4|TCP6|UNKNOWN <src> <dst> <srcport> <dstport>\r\n`
pub(crate) async fn parse_v1<R: AsyncReadExt + Unpin>(
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
    "TCP4" | "TCP6" => {
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

      // PROXY v1 only carries TCP4/TCP6 in this arm — `Udp` was dead
      // code (the pattern already restricted `proto` to "TCP*"). UDP is
      // a PROXY v2 concept; keep `Tcp` hard-coded for the v1 path.
      let mut header = ProxyHeader::empty(ProxyVersion::V1, ProxyTransport::Tcp);
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
