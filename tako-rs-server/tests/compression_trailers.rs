#![cfg(feature = "plugins")]

use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use tako::plugins::compression::CompressionBuilder;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;

fn router() -> Router {
  let mut router = Router::new();
  router.plugin(CompressionBuilder::new().min_size(0).build());
  router.get("/plain", || async { "first second" });
  router.get("/", || async {
    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-final-state", "complete".parse().unwrap());
    let body = TakoBody::new(
      TakoBody::from(Bytes::from_static(b"first second"))
        .with_trailers(std::future::ready(Some(Ok(trailers)))),
    );
    http::Response::builder()
      .header("content-type", "text/plain")
      .header("trailer", "x-final-state")
      .body(body)
      .unwrap()
  });
  router
}

async fn client(address: SocketAddr) {
  let stream = tokio::net::TcpStream::connect(address).await.unwrap();
  let (mut sender, connection) =
    hyper::client::conn::http1::handshake(hyper_util::rt::TokioIo::new(stream))
      .await
      .unwrap();
  let connection = tokio::spawn(connection);
  for encoding in ["gzip", "br", "deflate"] {
    let response = sender
      .send_request(
        http::Request::builder()
          .uri("/")
          .header("accept-encoding", encoding)
          .header("connection", "te")
          .header("te", "trailers")
          .body(TakoBody::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(response.headers()["content-encoding"], encoding);
    let content_length = response.headers().get("content-length").cloned();
    let collected = response.into_body().collect().await.unwrap();
    assert_eq!(
      collected
        .trailers()
        .and_then(|trailers| trailers.get("x-final-state")),
      Some(&http::HeaderValue::from_static("complete")),
      "{encoding}: HTTP/1 dropped buffered response trailers; Content-Length={content_length:?}"
    );
    assert!(
      content_length.is_none(),
      "{encoding}: trailers require chunked framing"
    );
    assert!(!collected.to_bytes().is_empty());
    let plain = sender
      .send_request(
        http::Request::builder()
          .uri("/plain")
          .header("accept-encoding", encoding)
          .body(TakoBody::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert!(
      plain.headers().contains_key("content-length"),
      "{encoding}: trailer-free output must retain its known length"
    );
    let plain = plain.into_body().collect().await.unwrap();
    assert!(plain.trailers().is_none());
  }
  drop(sender);
  connection.await.unwrap().unwrap();
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn buffered_compression_delivers_http1_trailers() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = tako_rs_server::Server::builder()
    .build()
    .spawn_http(listener, router());
  tokio::time::timeout(Duration::from_secs(5), client(address))
    .await
    .unwrap();
  server.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "compio")]
#[test]
fn buffered_compression_delivers_http1_trailers() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tako_rs_server::CompioServer::builder()
      .build()
      .spawn_http(listener, router());
    let (done, finished) = tokio::sync::oneshot::channel();
    let thread = std::thread::spawn(move || {
      tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(client(address));
      let _ = done.send(());
    });
    let result = compio::time::timeout(Duration::from_secs(5), finished).await;
    server.shutdown(Duration::from_secs(1)).await;
    thread.join().unwrap();
    result.unwrap().unwrap();
  });
}
