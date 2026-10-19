use super::WebSocketUpgrade;
use crate::body::TakoBody;
use crate::types::Request;

fn request() -> Request {
  http::Request::builder()
    .header("host", "localhost")
    .header("connection", "upgrade")
    .header("upgrade", "websocket")
    .header("sec-websocket-version", "13")
    .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
    .body(TakoBody::empty())
    .unwrap()
}

#[test]
fn invalid_buffer_sizes_are_rejected_before_upgrading() {
  for size in [0, 65_537, usize::MAX] {
    let (response, session) = WebSocketUpgrade::new(request(), |_| async {})
      .read_buffer_size(size)
      .accept();
    assert_eq!(
      response.status(),
      http::StatusCode::INTERNAL_SERVER_ERROR,
      "{size}"
    );
    assert!(session.is_none());
  }
}

#[test]
fn unrepresentable_limits_are_rejected_before_upgrading() {
  let upgrades = [
    WebSocketUpgrade::new(request(), handler).max_frame_size(usize::MAX),
    WebSocketUpgrade::new(request(), handler).max_message_size(usize::MAX),
    WebSocketUpgrade::new(request(), handler).write_buffer_size(usize::MAX),
  ];
  for upgrade in upgrades {
    let (response, session) = upgrade.accept();
    assert_eq!(response.status(), http::StatusCode::INTERNAL_SERVER_ERROR);
    assert!(session.is_none());
  }
}

async fn handler(_: crate::ws::WebSocket) {}

#[test]
fn a_protocol_from_a_later_header_is_returned_in_the_upgrade() {
  let mut request = request();
  request.headers_mut().append(
    http::header::SEC_WEBSOCKET_PROTOCOL,
    http::HeaderValue::from_static("other"),
  );
  request.headers_mut().append(
    http::header::SEC_WEBSOCKET_PROTOCOL,
    http::HeaderValue::from_static("chat"),
  );
  let (response, _) = WebSocketUpgrade::new(request, handler)
    .protocols(["chat"])
    .accept();
  assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);
  assert_eq!(
    response.headers().get(http::header::SEC_WEBSOCKET_PROTOCOL),
    Some(&http::HeaderValue::from_static("chat"))
  );
}

#[test]
fn zero_payload_limits_and_write_through_are_valid() {
  let (response, _) = WebSocketUpgrade::new(request(), handler)
    .max_frame_size(0)
    .max_message_size(0)
    .read_buffer_size(1)
    .write_buffer_size(0)
    .accept();
  assert_eq!(response.status(), http::StatusCode::SWITCHING_PROTOCOLS);
}
