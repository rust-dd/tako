#![cfg(all(feature = "http2", not(feature = "compio")))]

use std::time::Duration;

use http::StatusCode;
use http::Version;
use http_body_util::BodyExt;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_server::ServerConfig;
use tako_rs_server::serve_h2c_with_shutdown_and_config;
use tokio::net::TcpListener;
use tokio::net::TcpStream;

#[tokio::test]
async fn h2c_keep_alive_has_a_timer_and_serves_requests() {
  tokio::time::timeout(Duration::from_secs(3), async {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let mut router = Router::new();
    router.get("/ping", || async { "ok" });
    let config = ServerConfig {
      h2_keep_alive_interval: Some(Duration::from_millis(10)),
      drain_timeout: Duration::from_millis(100),
      ..ServerConfig::default()
    };
    let (shutdown, signal) = tokio::sync::oneshot::channel();
    let server = tokio::spawn(serve_h2c_with_shutdown_and_config(
      listener,
      router,
      async {
        let _ = signal.await;
      },
      config,
    ));
    let stream = TcpStream::connect(address).await.unwrap();
    let (mut client, connection) =
      hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
        .await
        .unwrap();
    let connection = tokio::spawn(connection);
    for _ in 0..2 {
      let request = http::Request::builder()
        .uri("http://localhost/ping")
        .body(TakoBody::empty())
        .unwrap();
      let response = client.send_request(request).await.unwrap();
      assert_eq!(response.status(), StatusCode::OK);
      assert_eq!(response.version(), Version::HTTP_2);
      assert_eq!(
        response.into_body().collect().await.unwrap().to_bytes(),
        "ok"
      );
      tokio::time::sleep(Duration::from_millis(30)).await;
    }
    drop(client);
    connection.await.unwrap().unwrap();
    shutdown.send(()).unwrap();
    server.await.unwrap();
  })
  .await
  .expect("HTTP/2 keep-alive test timed out");
}
