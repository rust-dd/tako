use bytes::BufMut;
use bytes::Bytes;
use bytes::BytesMut;

use super::CloseCode;
use super::CloseFrame;
use super::Error;
use super::Message;
use super::ProtocolError;
use super::Utf8Bytes;
use super::frame::OpCode;
use super::frame::encode_header;
use super::receiver::Frame;
use super::receiver::Limits;
use super::receiver::Receiver;

const MAX_CONTROL_PAYLOAD: usize = 125;
const MAX_CLOSE_REASON: usize = MAX_CONTROL_PAYLOAD - 2;

#[cfg(test)]
mod regression_tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum State {
  Open,
  CloseSent,
  CloseReceived,
  Closed,
}

/// Connection-level protocol: close handshake, automatic replies and frame encoding.
pub(crate) struct Protocol {
  receiver: Receiver,
  state: State,
  pending_pong: Option<Bytes>,
  pending_close: Option<Bytes>,
}

impl Protocol {
  pub(crate) fn new(limits: Limits) -> Self {
    Self {
      receiver: Receiver::new(limits),
      state: State::Open,
      pending_pong: None,
      pending_close: None,
    }
  }

  pub(crate) fn state(&self) -> State {
    self.state
  }

  pub(crate) fn wanted(&self, buf: &BytesMut) -> usize {
    self.receiver.wanted(buf)
  }

  /// Next message for the user; queues the automatic Pong and Close replies.
  pub(crate) fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Message>, ProtocolError> {
    if matches!(self.state, State::CloseReceived | State::Closed) {
      return Ok(None);
    }
    let Some(frame) = self.receiver.decode(buf)? else {
      return Ok(None);
    };
    Ok(Some(match frame {
      Frame::Message(message) => message,
      Frame::Ping(data) => {
        // A newer ping replaces a waiting pong: while the write buffer is full, one pong waits.
        if self.state == State::Open {
          self.pending_pong = Some(data.clone());
        }
        Message::Ping(data)
      }
      Frame::Pong(data) => Message::Pong(data),
      Frame::Close(close) => self.peer_close(close),
    }))
  }

  /// Encodes a user message; a Close starts the closing handshake.
  pub(crate) fn encode(&mut self, message: Message, out: &mut BytesMut) -> Result<(), Error> {
    if self.state != State::Open {
      return if message.is_close() {
        Ok(())
      } else {
        Err(Error::Closed)
      };
    }
    match message {
      Message::Text(text) => put_frame(out, OpCode::Text, text.as_bytes()),
      Message::Binary(data) => put_frame(out, OpCode::Binary, &data),
      Message::Ping(data) => put_frame(out, OpCode::Ping, control_payload(&data)?),
      Message::Pong(data) => put_frame(out, OpCode::Pong, control_payload(&data)?),
      Message::Close(frame) => {
        if let Some(frame) = &frame {
          if !frame.code.is_allowed() {
            return Err(ProtocolError::InvalidCloseFrame.into());
          }
          if frame.reason.len() > MAX_CLOSE_REASON {
            return Err(ProtocolError::ControlFrameTooLarge.into());
          }
        }
        self.pending_pong = None;
        put_close(out, frame.as_ref());
        self.state = State::CloseSent;
      }
    }
    Ok(())
  }

  pub(crate) fn drain_pong(&mut self, out: &mut BytesMut) {
    if let Some(data) = self.pending_pong.take() {
      put_frame(out, OpCode::Pong, &data);
    }
  }

  pub(crate) fn drain_close(&mut self, out: &mut BytesMut) {
    if let Some(close) = self.pending_close.take() {
      out.extend_from_slice(&close);
      if self.state == State::CloseReceived {
        self.state = State::Closed;
      }
    }
  }

  /// Queues Close with the error's code and stops the connection.
  pub(crate) fn fail(&mut self, error: ProtocolError) {
    if self.state == State::Open {
      self.pending_close = Some(encoded_close(error.close_code()));
    }
    self.pending_pong = None;
    self.state = State::Closed;
  }

  /// Errors with `ResetWithoutClosingHandshake` unless the closing handshake finished.
  pub(crate) fn eof(&mut self) -> Result<(), ProtocolError> {
    let state = std::mem::replace(&mut self.state, State::Closed);
    match state {
      State::CloseReceived | State::Closed => Ok(()),
      State::Open | State::CloseSent => Err(ProtocolError::ResetWithoutClosingHandshake),
    }
  }

  pub(crate) fn close_if_open(&mut self, code: CloseCode) {
    if self.state == State::Open {
      self.pending_pong = None;
      self.pending_close = Some(encoded_close(code));
      self.state = State::CloseSent;
    }
  }

  fn peer_close(&mut self, close: Option<CloseFrame>) -> Message {
    self.pending_pong = None;
    if self.state == State::CloseSent {
      self.state = State::Closed;
      return Message::Close(close);
    }
    let close = close.map(|frame| {
      if frame.code.is_allowed() {
        frame
      } else {
        CloseFrame {
          code: CloseCode::PROTOCOL_ERROR,
          reason: Utf8Bytes::from_static("Protocol violation"),
        }
      }
    });
    let mut reply = BytesMut::new();
    put_close(&mut reply, close.as_ref());
    self.pending_close = Some(reply.freeze());
    self.state = State::CloseReceived;
    Message::Close(close)
  }
}

fn put_frame(out: &mut BytesMut, opcode: OpCode, payload: &[u8]) {
  encode_header(out, opcode, payload.len());
  out.extend_from_slice(payload);
}

fn put_close(out: &mut BytesMut, frame: Option<&CloseFrame>) {
  let Some(frame) = frame else {
    encode_header(out, OpCode::Close, 0);
    return;
  };
  let reason = frame.reason.as_str();
  encode_header(out, OpCode::Close, 2 + reason.len());
  out.put_u16(frame.code.as_u16());
  out.extend_from_slice(reason.as_bytes());
}

fn encoded_close(code: CloseCode) -> Bytes {
  let mut out = BytesMut::new();
  put_close(
    &mut out,
    Some(&CloseFrame {
      code,
      reason: Utf8Bytes::default(),
    }),
  );
  out.freeze()
}

fn control_payload(data: &[u8]) -> Result<&[u8], ProtocolError> {
  if data.len() > MAX_CONTROL_PAYLOAD {
    return Err(ProtocolError::ControlFrameTooLarge);
  }
  Ok(data)
}

#[cfg(test)]
mod tests {
  use bytes::Bytes;
  use bytes::BytesMut;

  use super::Protocol;
  use super::State;
  use crate::ws::CloseCode;
  use crate::ws::Error;
  use crate::ws::Message;
  use crate::ws::ProtocolError;
  use crate::ws::receiver::Limits;
  use crate::ws::testing::CLOSE;
  use crate::ws::testing::PING;
  use crate::ws::testing::client_frame;
  use crate::ws::testing::close_payload;

  const LIMITS: Limits = Limits {
    max_frame_size: 1 << 20,
    max_message_size: 1 << 20,
  };

  fn drained(protocol: &mut Protocol) -> Vec<u8> {
    let mut out = BytesMut::new();
    protocol.drain_pong(&mut out);
    protocol.drain_close(&mut out);
    out.to_vec()
  }

  fn server_close(code: u16, reason: &str) -> Vec<u8> {
    let mut frame = vec![0x88, (2 + reason.len()) as u8];
    frame.extend_from_slice(&code.to_be_bytes());
    frame.extend_from_slice(reason.as_bytes());
    frame
  }

  #[test]
  fn a_ping_is_answered_with_its_payload() {
    let mut protocol = Protocol::new(LIMITS);
    let mut buf = BytesMut::from(&client_frame(PING, b"abc")[..]);
    let message = protocol.decode(&mut buf).unwrap();
    assert_eq!(message, Some(Message::Ping(Bytes::from_static(b"abc"))));
    assert_eq!(drained(&mut protocol), [0x8A, 3, b'a', b'b', b'c']);
  }

  #[test]
  fn a_ping_flood_leaves_one_pending_pong() {
    let mut protocol = Protocol::new(LIMITS);
    let mut buf = BytesMut::new();
    for i in 0..1000u16 {
      buf.extend(client_frame(PING, &i.to_be_bytes()));
    }
    while protocol.decode(&mut buf).unwrap().is_some() {}
    let last = 999u16.to_be_bytes();
    assert_eq!(drained(&mut protocol), [0x8A, 2, last[0], last[1]]);
  }

  #[test]
  fn a_peer_close_is_echoed_and_ends_reading() {
    let mut protocol = Protocol::new(LIMITS);
    let mut buf = BytesMut::from(&client_frame(CLOSE, &close_payload(1001, "bye"))[..]);
    buf.extend(client_frame(PING, b"late"));
    let Some(Message::Close(Some(frame))) = protocol.decode(&mut buf).unwrap() else {
      panic!("expected a close frame");
    };
    assert_eq!(frame.code, CloseCode::GOING_AWAY);
    assert_eq!(protocol.state(), State::CloseReceived);
    assert_eq!(protocol.decode(&mut buf).unwrap(), None);
    assert_eq!(drained(&mut protocol), server_close(1001, "bye"));
    assert_eq!(protocol.state(), State::Closed);
  }

  #[test]
  fn a_close_code_no_peer_may_send_is_answered_with_1002() {
    let mut protocol = Protocol::new(LIMITS);
    let mut buf = BytesMut::from(&client_frame(CLOSE, &close_payload(1005, ""))[..]);
    protocol.decode(&mut buf).unwrap();
    assert_eq!(
      drained(&mut protocol),
      server_close(1002, "Protocol violation")
    );
  }

  #[test]
  fn our_close_then_the_reply_closes_the_connection() {
    let mut protocol = Protocol::new(LIMITS);
    let mut out = BytesMut::new();
    protocol.encode(Message::Close(None), &mut out).unwrap();
    assert_eq!(&out[..], &[0x88, 0]);
    assert_eq!(protocol.state(), State::CloseSent);
    assert!(matches!(
      protocol.encode(Message::text("late"), &mut out),
      Err(Error::Closed)
    ));
    let mut buf = BytesMut::from(&client_frame(CLOSE, &close_payload(1000, ""))[..]);
    assert!(protocol.decode(&mut buf).unwrap().unwrap().is_close());
    assert_eq!(protocol.state(), State::Closed);
    assert_eq!(drained(&mut protocol), Vec::<u8>::new());
  }

  #[test]
  fn a_protocol_error_queues_its_close_code() {
    let mut protocol = Protocol::new(LIMITS);
    protocol.fail(ProtocolError::InvalidUtf8);
    assert_eq!(drained(&mut protocol), server_close(1007, ""));
    assert_eq!(protocol.state(), State::Closed);
  }

  #[test]
  fn eof_is_an_error_only_before_a_closing_handshake() {
    let mut open = Protocol::new(LIMITS);
    assert_eq!(open.eof(), Err(ProtocolError::ResetWithoutClosingHandshake));
    let mut closing = Protocol::new(LIMITS);
    let mut buf = BytesMut::from(&client_frame(CLOSE, &[])[..]);
    closing.decode(&mut buf).unwrap();
    assert_eq!(closing.eof(), Ok(()));
  }

  #[test]
  fn close_if_open_only_closes_once() {
    let mut protocol = Protocol::new(LIMITS);
    protocol.close_if_open(CloseCode::NORMAL);
    protocol.close_if_open(CloseCode::INTERNAL);
    assert_eq!(drained(&mut protocol), server_close(1000, ""));
  }
}
