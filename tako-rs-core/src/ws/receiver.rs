use bytes::Buf;
use bytes::Bytes;
use bytes::BytesMut;

use super::CloseCode;
use super::CloseFrame;
use super::Message;
use super::ProtocolError;
use super::Utf8Bytes;
use super::frame::Header;
use super::frame::OpCode;
use super::frame::parse_header;
use super::mask::unmask;
use super::utf8::Utf8Validator;

// A frame is read at most this far ahead, so its header alone cannot allocate it.
pub(crate) const MAX_READ_AHEAD: usize = 64 * 1024;

// Small payloads are copied out so a stored message never pins the read buffer.
const COPY_THRESHOLD: usize = 4 * 1024;

#[derive(Clone, Copy, Debug)]
pub(crate) struct Limits {
  pub max_frame_size: usize,
  pub max_message_size: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Frame {
  Message(Message),
  Ping(Bytes),
  Pong(Bytes),
  Close(Option<CloseFrame>),
}

struct Partial {
  text: bool,
  data: BytesMut,
  utf8: Utf8Validator,
}

/// Turns client frames buffered in a `BytesMut` into messages and control frames.
pub(crate) struct Receiver {
  limits: Limits,
  header: Option<Header>,
  partial: Option<Partial>,
}

impl Receiver {
  pub(crate) fn new(limits: Limits) -> Self {
    Self {
      limits,
      header: None,
      partial: None,
    }
  }

  /// Next complete frame from `buf`, consuming its bytes. `Ok(None)` when more input is needed.
  pub(crate) fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Frame>, ProtocolError> {
    loop {
      let header = if let Some(header) = self.header {
        header
      } else {
        let Some(header) = parse_header(buf, self.limits.max_frame_size)? else {
          return Ok(None);
        };
        self.check_order(&header)?;
        buf.advance(header.header_len);
        self.header = Some(header);
        header
      };
      let len = header.len as usize;
      if buf.len() < len {
        return Ok(None);
      }
      self.header = None;
      let payload = take_payload(buf, len, header.mask);
      if let Some(frame) = self.frame(header, payload)? {
        return Ok(Some(frame));
      }
    }
  }

  /// Payload bytes the frame in progress still needs; 0 when no header is pending.
  pub(crate) fn wanted(&self, buf: &BytesMut) -> usize {
    self
      .header
      .map_or(0, |header| (header.len as usize).saturating_sub(buf.len()))
  }

  fn check_order(&self, header: &Header) -> Result<(), ProtocolError> {
    let max = self.limits.max_message_size;
    let len = header.len as usize;
    match (header.opcode, &self.partial) {
      (OpCode::Continuation, None) => Err(ProtocolError::UnexpectedContinuation),
      (OpCode::Text | OpCode::Binary, Some(_)) => Err(ProtocolError::ExpectedContinuation),
      (OpCode::Continuation, Some(partial))
        if partial
          .data
          .len()
          .checked_add(len)
          .is_none_or(|size| size > max) =>
      {
        Err(ProtocolError::MessageTooLarge {
          size: partial.data.len().saturating_add(len),
          max,
        })
      }
      (OpCode::Text | OpCode::Binary, None) if len > max => {
        Err(ProtocolError::MessageTooLarge { size: len, max })
      }
      _ => Ok(()),
    }
  }

  fn frame(&mut self, header: Header, payload: Bytes) -> Result<Option<Frame>, ProtocolError> {
    match header.opcode {
      OpCode::Ping => Ok(Some(Frame::Ping(payload))),
      OpCode::Pong => Ok(Some(Frame::Pong(payload))),
      OpCode::Close => close_frame(payload).map(|close| Some(Frame::Close(close))),
      OpCode::Text if header.fin => Utf8Bytes::try_from(payload)
        .map(|text| Some(Frame::Message(Message::Text(text))))
        .map_err(|_| ProtocolError::InvalidUtf8),
      OpCode::Binary if header.fin => Ok(Some(Frame::Message(Message::Binary(payload)))),
      OpCode::Text | OpCode::Binary => {
        let mut partial = Partial {
          text: header.opcode == OpCode::Text,
          data: BytesMut::with_capacity(payload.len()),
          utf8: Utf8Validator::default(),
        };
        partial.push(&payload)?;
        self.partial = Some(partial);
        Ok(None)
      }
      OpCode::Continuation => {
        let Some(partial) = self.partial.as_mut() else {
          return Err(ProtocolError::UnexpectedContinuation);
        };
        partial.push(&payload)?;
        if !header.fin {
          return Ok(None);
        }
        let Some(partial) = self.partial.take() else {
          return Err(ProtocolError::UnexpectedContinuation);
        };
        partial
          .finish()
          .map(|message| Some(Frame::Message(message)))
      }
    }
  }
}

impl Partial {
  fn push(&mut self, payload: &[u8]) -> Result<(), ProtocolError> {
    if self.text {
      self.utf8.feed(payload)?;
    }
    self.data.extend_from_slice(payload);
    Ok(())
  }

  fn finish(self) -> Result<Message, ProtocolError> {
    if !self.text {
      return Ok(Message::Binary(self.data.freeze()));
    }
    self.utf8.finish()?;
    // SAFETY: every fragment went through the validator and `finish` saw no cut character.
    Ok(Message::Text(unsafe {
      Utf8Bytes::from_bytes_unchecked(self.data.freeze())
    }))
  }
}

fn take_payload(buf: &mut BytesMut, len: usize, mask: [u8; 4]) -> Bytes {
  if len <= COPY_THRESHOLD {
    let mut payload = buf[..len].to_vec();
    buf.advance(len);
    unmask(&mut payload, mask);
    Bytes::from(payload)
  } else {
    let mut payload = buf.split_to(len);
    if len > MAX_READ_AHEAD {
      // The payload keeps the frame's allocation, so the rest moves out instead of pinning it.
      *buf = BytesMut::from(&buf[..]);
    }
    unmask(&mut payload, mask);
    payload.freeze()
  }
}

fn close_frame(payload: Bytes) -> Result<Option<CloseFrame>, ProtocolError> {
  match *payload {
    [] => Ok(None),
    [_] => Err(ProtocolError::InvalidCloseFrame),
    [high, low, ..] => {
      let reason =
        Utf8Bytes::try_from(payload.slice(2..)).map_err(|_| ProtocolError::InvalidUtf8)?;
      Ok(Some(CloseFrame {
        code: CloseCode::new(u16::from_be_bytes([high, low])),
        reason,
      }))
    }
  }
}

#[cfg(test)]
mod tests {
  use bytes::Bytes;
  use bytes::BytesMut;

  use super::Frame;
  use super::Limits;
  use super::Receiver;
  use crate::ws::CloseCode;
  use crate::ws::CloseFrame;
  use crate::ws::Message;
  use crate::ws::ProtocolError;
  use crate::ws::Utf8Bytes;
  use crate::ws::testing::BINARY;
  use crate::ws::testing::CLOSE;
  use crate::ws::testing::CONTINUE;
  use crate::ws::testing::CONTINUE_END;
  use crate::ws::testing::PING;
  use crate::ws::testing::PONG;
  use crate::ws::testing::TEXT;
  use crate::ws::testing::TEXT_START;
  use crate::ws::testing::client_frame;
  use crate::ws::testing::close_payload;

  const LIMITS: Limits = Limits {
    max_frame_size: 16 << 20,
    max_message_size: 64 << 20,
  };

  fn decode_all(receiver: &mut Receiver, bytes: &[u8]) -> Result<Vec<Frame>, ProtocolError> {
    let mut buf = BytesMut::from(bytes);
    let mut frames = Vec::new();
    while let Some(frame) = receiver.decode(&mut buf)? {
      frames.push(frame);
    }
    assert!(buf.is_empty(), "left {} bytes", buf.len());
    Ok(frames)
  }

  #[test]
  fn decodes_each_kind_of_frame() {
    let mut bytes = client_frame(TEXT, b"hello");
    bytes.extend(client_frame(BINARY, &[1, 2, 3]));
    bytes.extend(client_frame(PING, b"p"));
    bytes.extend(client_frame(PONG, b"q"));
    bytes.extend(client_frame(CLOSE, &close_payload(1000, "bye")));
    bytes.extend(client_frame(CLOSE, &[]));
    let frames = decode_all(&mut Receiver::new(LIMITS), &bytes).unwrap();
    assert_eq!(
      frames,
      vec![
        Frame::Message(Message::text("hello")),
        Frame::Message(Message::Binary(Bytes::from_static(&[1, 2, 3]))),
        Frame::Ping(Bytes::from_static(b"p")),
        Frame::Pong(Bytes::from_static(b"q")),
        Frame::Close(Some(CloseFrame {
          code: CloseCode::NORMAL,
          reason: Utf8Bytes::from_static("bye"),
        })),
        Frame::Close(None),
      ]
    );
  }

  #[test]
  fn sixteen_buffered_frames_decode_one_by_one() {
    let bytes: Vec<u8> = (0..16).flat_map(|_| client_frame(TEXT, b"hello")).collect();
    let frames = decode_all(&mut Receiver::new(LIMITS), &bytes).unwrap();
    assert_eq!(frames.len(), 16);
  }

  #[test]
  fn a_frame_decodes_once_its_last_byte_arrives() {
    let payload: Vec<u8> = (0..300u16).map(|i| i as u8).collect();
    let bytes = client_frame(BINARY, &payload);
    for split in 0..bytes.len() {
      let mut receiver = Receiver::new(LIMITS);
      let mut buf = BytesMut::from(&bytes[..split]);
      assert_eq!(receiver.decode(&mut buf), Ok(None), "{split}");
      buf.extend_from_slice(&bytes[split..]);
      let frame = receiver.decode(&mut buf).unwrap();
      assert_eq!(
        frame,
        Some(Frame::Message(Message::binary(payload.clone())))
      );
    }
  }

  #[test]
  fn one_byte_text_fragments_reassemble_around_a_ping() {
    let text = "héllo".as_bytes();
    let mut bytes = client_frame(TEXT_START, &text[..1]);
    for (i, byte) in text.iter().enumerate().skip(1) {
      if i == 2 {
        bytes.extend(client_frame(PING, b"mid"));
      }
      let first = if i == text.len() - 1 {
        CONTINUE_END
      } else {
        CONTINUE
      };
      bytes.extend(client_frame(first, std::slice::from_ref(byte)));
    }
    let frames = decode_all(&mut Receiver::new(LIMITS), &bytes).unwrap();
    assert_eq!(
      frames,
      vec![
        Frame::Ping(Bytes::from_static(b"mid")),
        Frame::Message(Message::text("héllo")),
      ]
    );
  }

  #[test]
  fn invalid_utf8_fails_in_the_fragment_that_carries_it() {
    let mut receiver = Receiver::new(LIMITS);
    let mut buf = BytesMut::from(&client_frame(TEXT_START, b"ok")[..]);
    assert_eq!(receiver.decode(&mut buf), Ok(None));
    buf.extend(client_frame(CONTINUE, &[0xC0, 0x80]));
    assert_eq!(receiver.decode(&mut buf), Err(ProtocolError::InvalidUtf8));
  }

  #[test]
  fn fragments_out_of_order_are_rejected() {
    let lone = client_frame(CONTINUE_END, b"x");
    assert_eq!(
      decode_all(&mut Receiver::new(LIMITS), &lone),
      Err(ProtocolError::UnexpectedContinuation)
    );
    let mut nested = client_frame(TEXT_START, b"a");
    nested.extend(client_frame(TEXT, b"b"));
    assert_eq!(
      decode_all(&mut Receiver::new(LIMITS), &nested),
      Err(ProtocolError::ExpectedContinuation)
    );
  }

  #[test]
  fn message_size_counts_every_fragment() {
    let limits = Limits {
      max_frame_size: 16,
      max_message_size: 20,
    };
    let mut bytes = client_frame(TEXT_START, &[b'a'; 12]);
    bytes.extend(client_frame(CONTINUE_END, &[b'b'; 12]));
    assert_eq!(
      decode_all(&mut Receiver::new(limits), &bytes),
      Err(ProtocolError::MessageTooLarge { size: 24, max: 20 })
    );
  }

  #[test]
  fn continuation_length_overflow_is_rejected() {
    let mut receiver = Receiver::new(Limits {
      max_frame_size: usize::MAX,
      max_message_size: usize::MAX,
    });
    let mut input = BytesMut::from(client_frame(TEXT_START, b"x").as_slice());
    assert_eq!(receiver.decode(&mut input), Ok(None));
    let header = super::Header {
      fin: true,
      opcode: super::OpCode::Continuation,
      mask: [0; 4],
      len: usize::MAX as u64,
      header_len: 14,
    };
    assert_eq!(
      receiver.check_order(&header),
      Err(ProtocolError::MessageTooLarge {
        size: usize::MAX,
        max: usize::MAX,
      })
    );
  }

  #[test]
  fn a_large_announced_frame_allocates_nothing_up_front() {
    let mut header = client_frame(BINARY, &[]);
    header[1] = 0xFF;
    header.splice(2..2, (16u64 << 20).to_be_bytes());
    let mut buf = BytesMut::with_capacity(64);
    buf.extend_from_slice(&header);
    buf.extend_from_slice(&[0; 10]);
    let mut receiver = Receiver::new(LIMITS);
    assert_eq!(receiver.decode(&mut buf), Ok(None));
    assert!(buf.capacity() < 128 * 1024, "{}", buf.capacity());
    assert_eq!(receiver.wanted(&buf), (16 << 20) - 10);
  }

  #[test]
  fn a_one_byte_close_payload_is_rejected() {
    let bytes = client_frame(CLOSE, &[3]);
    assert_eq!(
      decode_all(&mut Receiver::new(LIMITS), &bytes),
      Err(ProtocolError::InvalidCloseFrame)
    );
  }
}
