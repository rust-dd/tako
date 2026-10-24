#![cfg(feature = "proxy-protocol")]

use futures_util::FutureExt;
use tako_rs_server::proxy_protocol::ProxyHeader;
use tako_rs_server::proxy_protocol::ProxyTransport;
use tako_rs_server::proxy_protocol::read_proxy_protocol;

fn wire(command: u8, family_protocol: u8, payload: &[u8]) -> Vec<u8> {
  let mut wire = b"\r\n\r\n\0\r\nQUIT\n".to_vec();
  wire.extend_from_slice(&[0x20 | command, family_protocol]);
  wire.extend_from_slice(&(payload.len() as u16).to_be_bytes());
  wire.extend_from_slice(payload);
  wire
}

fn parse(bytes: &[u8]) -> std::io::Result<ProxyHeader> {
  read_proxy_protocol(&mut &bytes[..]).now_or_never().unwrap()
}

#[test]
fn reserved_commands_families_and_protocols_are_rejected() {
  for command in 2..=15 {
    assert!(
      parse(&wire(command, 0x11, &[0; 12])).is_err(),
      "command {command}"
    );
  }
  for family in 4..=15 {
    assert!(
      parse(&wire(1, family << 4 | 1, &[])).is_err(),
      "family {family}"
    );
  }
  for protocol in 3..=15 {
    assert!(
      parse(&wire(1, 0x10 | protocol, &[0; 12])).is_err(),
      "protocol {protocol}"
    );
  }
}

#[test]
fn address_blocks_require_the_complete_family_specific_length() {
  for (family, required) in [(1, 12), (2, 36), (3, 216)] {
    for protocol in [1, 2] {
      for size in [0, 1, required - 1] {
        assert!(
          parse(&wire(1, family << 4 | protocol, &vec![0; size])).is_err(),
          "family {family}, protocol {protocol}, size {size}"
        );
      }
      assert!(parse(&wire(1, family << 4 | protocol, &vec![0; required])).is_ok());
    }
  }
}

#[test]
fn local_ignores_the_family_protocol_and_payload() {
  let header = parse(&wire(0, 0xff, &[0x11; 12])).unwrap();
  assert_eq!(header.transport, ProxyTransport::Unknown);
  assert!(header.source.is_none());
  assert!(header.source_unix.is_none());
}

#[test]
fn unspecified_family_or_protocol_does_not_supply_an_address() {
  for family_protocol in [0x00, 0x01, 0x02, 0x10, 0x20, 0x30] {
    let header = parse(&wire(1, family_protocol, &[0x11; 216])).unwrap();
    assert_eq!(header.transport, ProxyTransport::Unknown);
    assert!(header.source.is_none());
    assert!(header.source_unix.is_none());
  }
}

#[test]
fn ipv4_ipv6_unix_and_tlvs_keep_their_decoded_values() {
  let mut ipv4 = vec![192, 0, 2, 1, 192, 0, 2, 2, 0, 80, 1, 187];
  ipv4.extend_from_slice(&[0x01, 0, 2, b'h', b'2']);
  let header = parse(&wire(1, 0x11, &ipv4)).unwrap();
  assert_eq!(header.source.unwrap().to_string(), "192.0.2.1:80");
  assert_eq!(header.destination.unwrap().to_string(), "192.0.2.2:443");
  assert_eq!(header.alpn.as_deref(), Some(b"h2".as_slice()));

  let mut ipv6 = vec![0; 36];
  ipv6[15] = 1;
  ipv6[31] = 2;
  ipv6[33] = 80;
  ipv6[34..36].copy_from_slice(&443u16.to_be_bytes());
  let header = parse(&wire(1, 0x22, &ipv6)).unwrap();
  assert_eq!(header.transport, ProxyTransport::Udp);
  assert_eq!(header.source.unwrap().to_string(), "[::1]:80");
  assert_eq!(header.destination.unwrap().to_string(), "[::2]:443");

  let mut unix = vec![0; 216];
  unix[..4].copy_from_slice(b"/src");
  unix[108..112].copy_from_slice(b"/dst");
  let header = parse(&wire(1, 0x31, &unix)).unwrap();
  assert_eq!(header.source_unix.unwrap().to_str(), Some("/src"));
  assert_eq!(header.destination_unix.unwrap().to_str(), Some("/dst"));
}

#[test]
fn crc_tlv_verifies_the_complete_header_after_validation() {
  let mut payload = vec![0; 12];
  payload.extend_from_slice(&[0x03, 0, 4, 0, 0, 0, 0]);
  let mut bytes = wire(1, 0x11, &payload);
  let checksum = crc32c::crc32c(&bytes);
  bytes[31..35].copy_from_slice(&checksum.to_be_bytes());
  assert_eq!(parse(&bytes).unwrap().crc32c_verified, Some(true));
  bytes[16] = 1;
  assert_eq!(parse(&bytes).unwrap().crc32c_verified, Some(false));
}

fn metadata_wire(command: u8, family_protocol: u8) -> Vec<u8> {
  let payload = [0x01, 0, 2, b'h', b'2', 0x03, 0, 4, 0, 0, 0, 0];
  let mut bytes = wire(command, family_protocol, &payload);
  let checksum = crc32c::crc32c(&bytes);
  bytes[24..28].copy_from_slice(&checksum.to_be_bytes());
  bytes
}

#[test]
fn unspecified_proxy_preserves_tlvs_and_verifies_crc_without_addresses() {
  for family_protocol in [0x00, 0x01, 0x02, 0x10, 0x20, 0x30] {
    let mut bytes = metadata_wire(1, family_protocol);
    let header = parse(&bytes).unwrap();
    assert_eq!(header.transport, ProxyTransport::Unknown);
    assert!(header.source.is_none());
    assert!(header.destination.is_none());
    assert!(header.source_unix.is_none());
    assert!(header.destination_unix.is_none());
    assert_eq!(header.alpn.as_deref(), Some(b"h2".as_slice()));
    assert_eq!(header.crc32c_verified, Some(true));
    assert_eq!(header.tlvs.len(), 2);

    bytes[20] = b'3';
    let corrupt = parse(&bytes).unwrap();
    assert_eq!(corrupt.alpn.as_deref(), Some(b"h3".as_slice()));
    assert_eq!(corrupt.crc32c_verified, Some(false));
  }
}

#[test]
fn local_ignores_tlvs_and_crc_even_when_the_payload_is_valid() {
  for family_protocol in [0x00, 0xff] {
    let header = parse(&metadata_wire(0, family_protocol)).unwrap();
    assert!(header.alpn.is_none());
    assert!(header.crc32c_verified.is_none());
    assert!(header.tlvs.is_empty());
    assert!(header.source.is_none());
    assert!(header.destination.is_none());
  }
}
