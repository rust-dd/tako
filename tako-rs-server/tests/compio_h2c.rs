#![cfg(all(feature = "compio", feature = "http2"))]

use std::sync::Arc;
use std::time::Duration;

use http::StatusCode;
use http::Version;
use http_body_util::BodyExt;
use hyper_util::rt::TokioExecutor;
use hyper_util::rt::TokioIo;
use tako_rs_core::body::TakoBody;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_server::CompioServer;
use tako_rs_server::ServerConfig;

fn h2c_get(address: std::net::SocketAddr, paths: &[&str]) -> Vec<(StatusCode, Version, String)> {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
  runtime.block_on(async {
    let stream = tokio::net::TcpStream::connect(address).await.unwrap();
    let (mut client, connection) =
      hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
        .await
        .unwrap();
    let connection = tokio::spawn(connection);
    let mut responses = Vec::new();
    for path in paths {
      let request = http::Request::builder()
        .uri(format!("http://localhost{path}"))
        .body(TakoBody::empty())
        .unwrap();
      let response = client.send_request(request).await.unwrap();
      let status = response.status();
      let version = response.version();
      let body = response.into_body().collect().await.unwrap().to_bytes();
      responses.push((status, version, String::from_utf8(body.to_vec()).unwrap()));
    }
    drop(client);
    connection.await.unwrap().unwrap();
    responses
  })
}

#[test]
fn compio_h2c_serves_prior_knowledge_requests() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let state = Arc::new(());
    let mut router = Router::new();
    router.with_state(state.clone());
    router.get("/ping", || async { "ok" });
    router.get("/transport", |req: Request| async move {
      let info = req.extensions().get::<ConnInfo>().unwrap();
      format!("{:?} tls={}", info.transport, info.tls.is_some())
    });
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = CompioServer::builder()
      .config(ServerConfig {
        h2_keep_alive_interval: Some(Duration::from_millis(10)),
        ..ServerConfig::default()
      })
      .build()
      .try_spawn_h2c(listener, router)
      .unwrap();
    assert_eq!(handle.local_addr(), Some(address));

    let (sender, received) = tokio::sync::oneshot::channel();
    let client = std::thread::spawn(move || {
      sender
        .send(h2c_get(address, &["/ping", "/transport", "/ping"]))
        .unwrap();
    });
    let responses = compio::time::timeout(Duration::from_secs(5), received)
      .await
      .expect("h2c client timed out")
      .unwrap();
    client.join().unwrap();

    assert_eq!(
      responses,
      vec![
        (StatusCode::OK, Version::HTTP_2, "ok".to_owned()),
        (
          StatusCode::OK,
          Version::HTTP_2,
          "Http2 tls=false".to_owned()
        ),
        (StatusCode::OK, Version::HTTP_2, "ok".to_owned()),
      ]
    );

    compio::time::timeout(
      Duration::from_secs(2),
      handle.shutdown(Duration::from_secs(1)),
    )
    .await
    .expect("h2c server did not shut down");
    assert_eq!(Arc::strong_count(&state), 1);
  });
}
