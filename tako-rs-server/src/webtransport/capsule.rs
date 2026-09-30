//! Session close signalling on the CONNECT stream
//! (draft-ietf-webtrans-http3, section 5).

use bytes::Buf;
use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;

use super::driver::ConnectRecv;

const CLOSE_WEBTRANSPORT_SESSION: u64 = 0x2843;
/// The draft caps the close reason at 1024 bytes.
const MAX_REASON_LEN: usize = 1024;
/// Capsules the server does not act on are skipped, but a client may not
/// make it buffer more than this much of one.
const MAX_CAPSULE_LEN: usize = 16 * 1024;

/// How a WebTransport session ended.
///
/// A client that closes without an error code reports code 0 and an empty
/// reason, as does a session that ends because its connection went away.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WebTransportClose {
  /// Application error code.
  pub code: u32,
  /// Human-readable reason, at most 1024 bytes.
  pub reason: String,
}

/// Encodes a `CLOSE_WEBTRANSPORT_SESSION` capsule, trimming the reason to
/// 1024 bytes on a character boundary.
pub(super) fn encode_close(code: u32, reason: &str) -> (Bytes, String) {
  let mut end = reason.len().min(MAX_REASON_LEN);
  while !reason.is_char_boundary(end) {
    end -= 1;
  }
  let reason = &reason[..end];
  let mut capsule = BytesMut::with_capacity(16 + reason.len());
  put_varint(&mut capsule, CLOSE_WEBTRANSPORT_SESSION);
  put_varint(&mut capsule, 4 + reason.len() as u64);
  capsule.put_u32(code);
  capsule.put_slice(reason.as_bytes());
  (capsule.freeze(), reason.to_owned())
}

/// Reads the CONNECT stream until the client ends the session: with a
/// `CLOSE_WEBTRANSPORT_SESSION` capsule, by finishing the stream, or by
/// resetting it.
pub(super) async fn watch(mut connect: ConnectRecv) -> WebTransportClose {
  let mut buffer = BytesMut::new();
  loop {
    match connect.recv_data().await {
      Ok(Some(mut data)) => {
        buffer.put(data.copy_to_bytes(data.remaining()));
        while let Some((kind, payload)) = next_capsule(&mut buffer) {
          if kind == CLOSE_WEBTRANSPORT_SESSION {
            return parse_close(payload);
          }
        }
        if buffer.len() > MAX_CAPSULE_LEN {
          return WebTransportClose::default();
        }
      }
      Ok(None) | Err(_) => return WebTransportClose::default(),
    }
  }
}

fn parse_close(mut payload: Bytes) -> WebTransportClose {
  if payload.remaining() < 4 {
    return WebTransportClose::default();
  }
  let code = payload.get_u32();
  WebTransportClose {
    code,
    reason: String::from_utf8_lossy(&payload).into_owned(),
  }
}

/// Splits one complete capsule off `buffer`, if it holds one.
fn next_capsule(buffer: &mut BytesMut) -> Option<(u64, Bytes)> {
  let mut peek = &buffer[..];
  let kind = get_varint(&mut peek)?;
  let len = usize::try_from(get_varint(&mut peek)?).ok()?;
  let header = buffer.len() - peek.len();
  if peek.len() < len {
    return None;
  }
  buffer.advance(header);
  Some((kind, buffer.split_to(len).freeze()))
}

fn get_varint(buf: &mut &[u8]) -> Option<u64> {
  let first = *buf.first()?;
  let len = 1usize << (first >> 6);
  if buf.len() < len {
    return None;
  }
  let mut value = u64::from(first & 0x3f);
  for byte in &buf[1..len] {
    value = (value << 8) | u64::from(*byte);
  }
  buf.advance(len);
  Some(value)
}

fn put_varint(buf: &mut BytesMut, value: u64) {
  match value {
    0..=0x3f => buf.put_u8(value as u8),
    0x40..=0x3fff => buf.put_u16(0x4000 | value as u16),
    0x4000..=0x3fff_ffff => buf.put_u32(0x8000_0000 | value as u32),
    _ => buf.put_u64(0xc000_0000_0000_0000 | value),
  }
}

#[cfg(test)]
mod tests {
  use bytes::BytesMut;

  use super::*;

  #[test]
  fn close_capsules_round_trip_and_trim_the_reason() {
    let (capsule, reason) = encode_close(7, "bye");
    assert_eq!(reason, "bye");
    let mut buffer = BytesMut::from(&capsule[..]);
    let (kind, payload) = next_capsule(&mut buffer).unwrap();
    assert_eq!(kind, CLOSE_WEBTRANSPORT_SESSION);
    assert!(buffer.is_empty());
    assert_eq!(
      parse_close(payload),
      WebTransportClose {
        code: 7,
        reason: "bye".into(),
      }
    );

    let (_, reason) = encode_close(0, &"é".repeat(600));
    assert!(reason.len() <= MAX_REASON_LEN && reason.chars().all(|c| c == 'é'));
  }

  #[test]
  fn partial_capsules_wait_for_the_rest() {
    let (capsule, _) = encode_close(1, "later");
    let mut buffer = BytesMut::from(&capsule[..capsule.len() - 1]);
    assert!(next_capsule(&mut buffer).is_none());
    buffer.extend_from_slice(&capsule[capsule.len() - 1..]);
    assert!(next_capsule(&mut buffer).is_some());
  }
}
