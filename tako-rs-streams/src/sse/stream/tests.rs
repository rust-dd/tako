use bytes::Bytes;
use futures_util::FutureExt;
use http_body_util::BodyExt;
use tako_rs_core::responder::Responder;

use super::Sse;

#[test]
fn sse_response_leaves_connection_management_to_the_transport() {
  let response =
    Sse::new(futures_util::stream::iter([Bytes::from_static(b"hello")])).into_response();
  assert_eq!(response.headers()["content-type"], "text/event-stream");
  assert!(!response.headers().contains_key("connection"));
  assert_eq!(
    response
      .into_body()
      .collect()
      .now_or_never()
      .unwrap()
      .unwrap()
      .to_bytes(),
    "data: hello\n\n"
  );
}
