#![cfg(any(feature = "http2", feature = "http3"))]

use tako::middleware::IntoMiddleware;
use tako::middleware::tenant::Tenant;
use tako::middleware::tenant::TenantMiddleware;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;

fn router() -> Router {
  let mut router = Router::new();
  router.middleware(
    TenantMiddleware::from_subdomain()
      .require(true)
      .into_middleware(),
  );
  router.get("/", |request: Request| async move {
    assert!(!request.headers().contains_key("host"));
    request.extensions().get::<Tenant>().unwrap().0.clone()
  });
  router
}

#[cfg(feature = "http3")]
#[path = "support/h3.rs"]
mod support;

#[cfg(feature = "http3")]
#[test]
fn h3_authority_without_host_resolves_the_tenant() {
  use bytes::Buf;

  support::run(router(), |mut client| async move {
    let mut request = client
      .sender
      .send_request(
        http::Request::builder()
          .uri("https://Acme.example.test/")
          .body(())
          .unwrap(),
      )
      .await
      .unwrap();
    request.finish().await.unwrap();
    assert_eq!(request.recv_response().await.unwrap().status(), 200);
    let mut data = request.recv_data().await.unwrap().unwrap();
    assert_eq!(data.copy_to_bytes(data.remaining()), "acme");
    assert!(request.recv_data().await.unwrap().is_none());
    drop(request);
    client.close().await;
  });
}

#[cfg(feature = "http2")]
async fn h2_request(address: std::net::SocketAddr) {
  use http_body_util::BodyExt;
  use hyper_util::rt::TokioExecutor;
  use hyper_util::rt::TokioIo;
  use tako_rs_core::body::TakoBody;

  let stream = tokio::net::TcpStream::connect(address).await.unwrap();
  let (mut client, driver) =
    hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(stream))
      .await
      .unwrap();
  let driver = tokio::spawn(driver);
  let response = client
    .send_request(
      http::Request::builder()
        .uri("http://Acme.example.test/")
        .body(TakoBody::empty())
        .unwrap(),
    )
    .await
    .unwrap();
  assert_eq!(response.status(), 200);
  assert_eq!(response.version(), http::Version::HTTP_2);
  assert_eq!(
    response.into_body().collect().await.unwrap().to_bytes(),
    "acme"
  );
  drop(client);
  driver.await.unwrap().unwrap();
}

#[cfg(all(feature = "http2", not(feature = "compio")))]
#[tokio::test]
async fn h2_authority_without_host_resolves_the_tenant() {
  use std::time::Duration;

  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = tako_rs_server::Server::builder()
    .build()
    .try_spawn_h2c(listener, router())
    .unwrap();
  tokio::time::timeout(Duration::from_secs(5), h2_request(address))
    .await
    .unwrap();
  server.shutdown(Duration::from_secs(1)).await;
}

#[cfg(all(feature = "http2", feature = "compio"))]
#[compio::test]
async fn h2_authority_without_host_resolves_the_tenant() {
  use std::time::Duration;

  let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let server = tako_rs_server::CompioServer::builder()
    .build()
    .try_spawn_h2c(listener, router())
    .unwrap();
  let (done, received) = tokio::sync::oneshot::channel();
  let client = std::thread::spawn(move || {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
      tokio::time::timeout(Duration::from_secs(5), h2_request(address))
        .await
        .unwrap();
    });
    let _ = done.send(());
  });
  let result = compio::time::timeout(Duration::from_secs(8), received).await;
  server.shutdown(Duration::from_secs(1)).await;
  client.join().unwrap();
  result.unwrap().unwrap();
}
