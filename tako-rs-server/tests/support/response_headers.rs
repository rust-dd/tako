use bytes::Bytes;
use http::HeaderMap;
use http::HeaderValue;
use http_body::Frame;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;

fn fields() -> HeaderMap {
  let mut headers = HeaderMap::new();
  headers.append(
    "connection",
    HeaderValue::from_static("keep-alive, X-First"),
  );
  headers.append("connection", HeaderValue::from_static("X-Second"));
  for (name, value) in [
    ("keep-alive", "timeout=5"),
    ("proxy-connection", "keep-alive"),
    ("transfer-encoding", "chunked"),
    ("upgrade", "websocket"),
    ("te", "trailers"),
    ("x-first", "private"),
    ("x-second", "private"),
    ("x-end-to-end", "preserved"),
  ] {
    headers.insert(name, HeaderValue::from_static(value));
  }
  headers
}

pub fn router() -> Router {
  let mut router = Router::new();
  router.get("/sse", || async {
    tako::sse::Sse::new(futures_util::stream::iter([Bytes::from_static(b"event")]))
  });
  router.get("/", || async {
    let mut trailers = fields();
    trailers.remove("connection");
    let frames = futures_util::stream::iter([
      Ok::<_, std::io::Error>(Frame::data(Bytes::from_static(b"event"))),
      Ok(Frame::trailers(trailers)),
    ]);
    let mut response = http::Response::new(TakoBody::from_try_stream(frames));
    *response.headers_mut() = fields();
    response
  });
  router
}

pub fn assert_clean(headers: &HeaderMap) {
  assert_eq!(headers["x-end-to-end"], "preserved");
  for name in [
    "connection",
    "keep-alive",
    "proxy-connection",
    "transfer-encoding",
    "upgrade",
    "te",
    "x-first",
    "x-second",
  ] {
    assert!(
      !headers.contains_key(name),
      "forbidden response field: {name}"
    );
  }
}
