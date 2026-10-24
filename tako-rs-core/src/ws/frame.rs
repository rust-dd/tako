use bytes::BufMut;
use bytes::BytesMut;

use super::ProtocolError;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OpCode {
  Continuation,
  Text,
  Binary,
  Close,
  Ping,
  Pong,
}

impl OpCode {
  pub(crate) fn is_control(self) -> bool {
    matches!(self, Self::Close | Self::Ping | Self::Pong)
  }

  fn from_bits(bits: u8) -> Result<Self, ProtocolError> {
    match bits {
      0 => Ok(Self::Continuation),
      1 => Ok(Self::Text),
      2 => Ok(Self::Binary),
      8 => Ok(Self::Close),
      9 => Ok(Self::Ping),
      10 => Ok(Self::Pong),
      other => Err(ProtocolError::UnknownOpcode(other)),
    }
  }

  fn bits(self) -> u8 {
    match self {
      Self::Continuation => 0,
      Self::Text => 1,
      Self::Binary => 2,
      Self::Close => 8,
      Self::Ping => 9,
      Self::Pong => 10,
    }
  }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Header {
  pub fin: bool,
  pub opcode: OpCode,
  pub mask: [u8; 4],
  pub len: u64,
  pub header_len: usize,
}

/// Parses a client frame header. `Ok(None)` while `buf` holds less than a whole header.
pub(crate) fn parse_header(
  buf: &[u8],
  max_frame_size: usize,
) -> Result<Option<Header>, ProtocolError> {
  let [first, second, ..] = *buf else {
    return Ok(None);
  };
  if first & 0x70 != 0 {
    return Err(ProtocolError::ReservedBits);
  }
  let fin = first & 0x80 != 0;
  let opcode = OpCode::from_bits(first & 0x0F)?;
  if second & 0x80 == 0 {
    return Err(ProtocolError::UnmaskedFrame);
  }
  let (len, offset) = match second & 0x7F {
    126 => {
      let Some(bytes) = array::<2>(buf, 2) else {
        return Ok(None);
      };
      let len = u64::from(u16::from_be_bytes(bytes));
      if len < 126 {
        return Err(ProtocolError::NonMinimalLength);
      }
      (len, 4)
    }
    127 => {
      let Some(bytes) = array::<8>(buf, 2) else {
        return Ok(None);
      };
      let len = u64::from_be_bytes(bytes);
      if len & (1 << 63) != 0 {
        return Err(ProtocolError::ReservedLengthBit);
      }
      if len < 65536 {
        return Err(ProtocolError::NonMinimalLength);
      }
      (len, 10)
    }
    short => (u64::from(short), 2),
  };
  if opcode.is_control() {
    if !fin {
      return Err(ProtocolError::FragmentedControlFrame);
    }
    if len > 125 {
      return Err(ProtocolError::ControlFrameTooLarge);
    }
  }
  if len > max_frame_size as u64 {
    return Err(ProtocolError::FrameTooLarge {
      size: len,
      max: max_frame_size,
    });
  }
  Ok(array::<4>(buf, offset).map(|mask| Header {
    fin,
    opcode,
    mask,
    len,
    header_len: offset + 4,
  }))
}

fn array<const N: usize>(buf: &[u8], at: usize) -> Option<[u8; N]> {
  buf.get(at..at + N)?.try_into().ok()
}

/// Writes the header of a final, unmasked server frame.
pub(crate) fn encode_header(out: &mut BytesMut, opcode: OpCode, len: usize) {
  out.put_u8(0x80 | opcode.bits());
  match len {
    0..=125 => out.put_u8(len as u8),
    126..=0xFFFF => {
      out.put_u8(126);
      out.put_u16(len as u16);
    }
    _ => {
      out.put_u8(127);
      out.put_u64(len as u64);
    }
  }
}

#[cfg(test)]
mod tests {
  use bytes::BytesMut;

  use super::Header;
  use super::OpCode;
  use super::ProtocolError;
  use super::encode_header;
  use super::parse_header;

  const MASK: [u8; 4] = [1, 2, 3, 4];
  const MAX: usize = 1 << 20;

  fn client_header(opcode: OpCode, len: usize) -> Vec<u8> {
    let mut out = BytesMut::new();
    encode_header(&mut out, opcode, len);
    let mut header = out.to_vec();
    header[1] |= 0x80;
    header.extend_from_slice(&MASK);
    header
  }

  #[test]
  fn every_length_encoding_parses_back() {
    for (len, header_len) in [(0, 6), (125, 6), (126, 8), (65535, 8), (65536, 14)] {
      let header = client_header(OpCode::Binary, len);
      assert_eq!(header.len(), header_len);
      let parsed = parse_header(&header, MAX).unwrap().unwrap();
      assert_eq!(
        parsed,
        Header {
          fin: true,
          opcode: OpCode::Binary,
          mask: MASK,
          len: len as u64,
          header_len,
        }
      );
    }
  }

  #[test]
  fn a_truncated_header_needs_more_bytes() {
    let header = client_header(OpCode::Text, 65536);
    for end in 0..header.len() {
      assert_eq!(parse_header(&header[..end], MAX), Ok(None), "{end}");
    }
  }

  #[test]
  fn header_rules_reject_bad_frames() {
    let check = |bytes: &[u8], expected: ProtocolError| {
      assert_eq!(parse_header(bytes, MAX), Err(expected), "{bytes:?}");
    };
    check(&[0xC2, 0x80, 1, 2, 3, 4], ProtocolError::ReservedBits);
    check(&[0x83, 0x80, 1, 2, 3, 4], ProtocolError::UnknownOpcode(3));
    check(&[0x8B, 0x80, 1, 2, 3, 4], ProtocolError::UnknownOpcode(11));
    check(&[0x82, 0x00], ProtocolError::UnmaskedFrame);
    check(
      &[0x09, 0x80, 1, 2, 3, 4],
      ProtocolError::FragmentedControlFrame,
    );
    check(&[0x89, 0xFE, 0, 126], ProtocolError::ControlFrameTooLarge);
  }

  #[test]
  fn an_oversized_frame_fails_on_its_header() {
    let header = client_header(OpCode::Binary, MAX + 1);
    assert_eq!(
      parse_header(&header, MAX),
      Err(ProtocolError::FrameTooLarge {
        size: MAX as u64 + 1,
        max: MAX,
      })
    );
  }

  #[test]
  fn non_minimal_lengths_are_protocol_errors() {
    for header in [
      vec![0x82, 0xfe, 0, 125, 1, 2, 3, 4],
      vec![0x82, 0xff, 0, 0, 0, 0, 0, 0, 0xff, 0xff, 1, 2, 3, 4],
    ] {
      let error = parse_header(&header, MAX).expect_err("length must use the shortest encoding");
      assert_eq!(error.close_code(), crate::ws::CloseCode::PROTOCOL_ERROR);
    }
  }

  #[test]
  fn a_64_bit_length_with_its_high_bit_set_is_a_protocol_error() {
    let header = [0x82, 0xff, 0x80, 0, 0, 0, 0, 0, 0, 0, 1, 2, 3, 4];
    let error = parse_header(&header, MAX).unwrap_err();
    assert_eq!(error.close_code(), crate::ws::CloseCode::PROTOCOL_ERROR);
  }

  #[test]
  fn minimal_extended_lengths_accept_fragmented_frames_at_the_size_limit() {
    for len in [126, 65535, 65536] {
      let mut header = client_header(OpCode::Binary, len);
      header[0] &= 0x7f;
      let parsed = parse_header(&header, len).unwrap().unwrap();
      assert!(!parsed.fin);
      assert_eq!(parsed.len, len as u64);
      assert!(matches!(
        parse_header(&header, len - 1),
        Err(ProtocolError::FrameTooLarge { .. })
      ));
    }
  }

  #[test]
  fn server_frames_are_unmasked_and_final() {
    let mut out = BytesMut::new();
    encode_header(&mut out, OpCode::Pong, 3);
    assert_eq!(&out[..], &[0x8A, 0x03]);
  }
}
