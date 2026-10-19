use bytes::Bytes;
use bytes::BytesMut;

use super::Protocol;
use super::State;
use crate::ws::CloseCode;
use crate::ws::CloseFrame;
use crate::ws::Error;
use crate::ws::Message;
use crate::ws::ProtocolError;
use crate::ws::receiver::Limits;
use crate::ws::testing::PING;
use crate::ws::testing::client_frame;

fn protocol() -> Protocol {
  Protocol::new(Limits {
    max_frame_size: 1024,
    max_message_size: 1024,
  })
}

#[test]
fn oversized_user_control_frames_are_rejected_without_changing_output() {
  for message in [
    Message::Ping(Bytes::from(vec![1; 126])),
    Message::Pong(Bytes::from(vec![2; 126])),
  ] {
    let mut protocol = protocol();
    let mut output = BytesMut::from(&b"queued"[..]);
    assert!(matches!(
      protocol.encode(message, &mut output),
      Err(Error::Protocol(ProtocolError::ControlFrameTooLarge))
    ));
    assert_eq!(&output[..], b"queued");
    assert_eq!(protocol.state(), State::Open);
  }
}

#[test]
fn invalid_user_close_codes_never_reach_the_wire() {
  for code in [0, 999, 1004, 1005, 1006, 1015, 5000] {
    let mut protocol = protocol();
    let mut output = BytesMut::new();
    let frame = CloseFrame {
      code: CloseCode::new(code),
      reason: "bye".into(),
    };
    assert!(
      matches!(
        protocol.encode(Message::Close(Some(frame)), &mut output),
        Err(Error::Protocol(ProtocolError::InvalidCloseFrame))
      ),
      "{code}"
    );
    assert!(output.is_empty());
    assert_eq!(protocol.state(), State::Open);
  }
}

#[test]
fn an_oversized_close_reason_is_rejected_instead_of_truncated() {
  let mut protocol = protocol();
  let mut output = BytesMut::new();
  let frame = CloseFrame {
    code: CloseCode::NORMAL,
    reason: "é".repeat(62).into(),
  };
  assert!(matches!(
    protocol.encode(Message::Close(Some(frame)), &mut output),
    Err(Error::Protocol(ProtocolError::ControlFrameTooLarge))
  ));
  assert!(output.is_empty());
  assert_eq!(protocol.state(), State::Open);
}

#[test]
fn a_pending_pong_cannot_follow_a_user_close() {
  let mut protocol = protocol();
  let mut input = BytesMut::from(client_frame(PING, b"p").as_slice());
  protocol.decode(&mut input).unwrap();
  let mut output = BytesMut::new();
  protocol.encode(Message::Close(None), &mut output).unwrap();
  protocol.drain_pong(&mut output);
  assert_eq!(&output[..], &[0x88, 0]);
}

#[test]
fn a_pending_pong_cannot_follow_the_drivers_close() {
  let mut protocol = protocol();
  let mut input = BytesMut::from(client_frame(PING, b"p").as_slice());
  protocol.decode(&mut input).unwrap();
  let mut output = BytesMut::new();
  protocol.close_if_open(CloseCode::NORMAL);
  protocol.drain_close(&mut output);
  protocol.drain_pong(&mut output);
  assert_eq!(&output[..], &[0x88, 2, 3, 232]);
}

#[test]
fn registered_gateway_close_code_is_accepted() {
  let mut protocol = protocol();
  let mut output = BytesMut::new();
  let frame = CloseFrame {
    code: CloseCode::new(1014),
    reason: "".into(),
  };
  protocol
    .encode(Message::Close(Some(frame)), &mut output)
    .unwrap();
  assert_eq!(&output[..], &[0x88, 2, 3, 246]);
}
