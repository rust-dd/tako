#![cfg(any(all(feature = "ws", not(feature = "compio")), feature = "compio-ws"))]

use http::StatusCode;
use http::header;
use tako::body::TakoBody;
use tako::responder::Responder;
use tako::types::Request;
use tako::types::Response;

fn respond(request: Request) -> Response {
  #[cfg(not(feature = "compio"))]
  {
    tako::ws::TakoWs::new(request, |_| async {}).into_response()
  }
  #[cfg(feature = "compio-ws")]
  {
    tako::ws_compio::TakoWsCompio::new(request, |_| async {}).into_response()
  }
}

fn request(version: &str) -> Request {
  http::Request::builder()
    .uri("/ws")
    .header(header::HOST, "localhost")
    .header(header::UPGRADE, "websocket")
    .header(header::CONNECTION, "upgrade")
    .header(header::SEC_WEBSOCKET_VERSION, version)
    .header(header::SEC_WEBSOCKET_KEY, "dGhlIHNhbXBsZSBub25jZQ==")
    .body(TakoBody::empty())
    .unwrap()
}

#[test]
fn each_responder_validates_before_accepting_the_upgrade() {
  let accepted = respond(request("13"));
  assert_eq!(accepted.status(), StatusCode::SWITCHING_PROTOCOLS);
  assert_eq!(
    accepted.headers()[header::SEC_WEBSOCKET_ACCEPT],
    "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
  );
  let rejected = respond(request("12"));
  assert_eq!(rejected.status(), StatusCode::UPGRADE_REQUIRED);
  assert_eq!(rejected.headers()[header::SEC_WEBSOCKET_VERSION], "13");
  let mut incomplete = request("13");
  incomplete.headers_mut().remove(header::UPGRADE);
  assert_eq!(respond(incomplete).status(), StatusCode::BAD_REQUEST);
}
