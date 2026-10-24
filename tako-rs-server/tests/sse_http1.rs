use std::net::SocketAddr;
use std::time::Duration;

use bytes::Bytes;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;

fn router() -> Router {
  let mut router = Router::new();
  router.get("/", || async {
    tako::sse::Sse::new(futures_util::stream::iter([Bytes::from_static(b"event")]))
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
  for _ in 0..2 {
    let response = sender
      .send_request(
        http::Request::builder()
          .uri("/")
          .body(TakoBody::empty())
          .unwrap(),
      )
      .await
      .unwrap();
    assert_eq!(response.headers()["content-type"], "text/event-stream");
    assert_eq!(
      response.into_body().collect().await.unwrap().to_bytes(),
      "data: event\n\n"
    );
  }
  drop(sender);
  connection.await.unwrap().unwrap();
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn http1_keeps_sse_connections_reusable() {
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
fn http1_keeps_sse_connections_reusable() {
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
