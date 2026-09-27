#![cfg(all(feature = "http3", not(feature = "compio")))]

use std::sync::Arc;
use std::time::Duration;

use bytes::Buf;
use bytes::Bytes;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_server::Server;
use tako_rs_server::ServerConfig;
use tako_rs_server::TlsCert;

#[tokio::test]
async fn h3_streams_data_and_trailers_and_cancels_pending_requests() {
  tokio::time::timeout(Duration::from_secs(5), exercise_h3())
    .await
    .expect("HTTP/3 lifecycle timed out");
}

async fn exercise_h3() {
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let marker = Arc::new(());
  let started = Arc::new(tokio::sync::Notify::new());
  let handler_marker = marker.clone();
  let handler_started = started.clone();
  let mut router = Router::new();
  router.post(
    "/echo",
    |request: Request| async move { request.into_body() },
  );
  router.get("/pending", move || {
    let marker = handler_marker.clone();
    let started = handler_started.clone();
    async move {
      started.notify_one();
      std::future::pending::<()>().await;
      drop(marker);
      "done"
    }
  });
  let handle = Server::builder()
    .tls(TlsCert::der(vec![certificate.clone()], key.into()))
    .config(ServerConfig {
      drain_timeout: Duration::from_millis(100),
      h3_goaway_grace: Duration::from_millis(20),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_h3("127.0.0.1:0", router)
    .unwrap();
  let mut roots = rustls::RootCertStore::empty();
  roots.add(certificate).unwrap();
  let mut tls = rustls::ClientConfig::builder()
    .with_root_certificates(roots)
    .with_no_client_auth();
  tls.alpn_protocols = vec![b"h3".to_vec()];
  let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap();
  let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
  endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic)));
  let connection = endpoint
    .connect(handle.local_addr().unwrap(), "localhost")
    .unwrap()
    .await
    .unwrap();
  let (mut driver, mut client) = h3::client::new(h3_quinn::Connection::new(connection))
    .await
    .unwrap();
  let driver_task =
    tokio::spawn(async move { std::future::poll_fn(|cx| driver.poll_close(cx)).await });
  let mut echo = client
    .send_request(
      http::Request::builder()
        .method(http::Method::POST)
        .uri("https://localhost/echo")
        .body(())
        .unwrap(),
    )
    .await
    .unwrap();
  echo.send_data(Bytes::from_static(b"first")).await.unwrap();
  assert_eq!(
    echo.recv_response().await.unwrap().status(),
    http::StatusCode::OK
  );
  let mut first = echo.recv_data().await.unwrap().unwrap();
  assert_eq!(first.copy_to_bytes(first.remaining()), "first");
  let mut trailers = http::HeaderMap::new();
  trailers.insert("x-checksum", "verified".parse().unwrap());
  echo.send_trailers(trailers).await.unwrap();
  echo.finish().await.unwrap();
  assert!(echo.recv_data().await.unwrap().is_none());
  assert_eq!(
    echo.recv_trailers().await.unwrap().unwrap()["x-checksum"],
    "verified"
  );
  let mut pending = client
    .send_request(
      http::Request::builder()
        .uri("https://localhost/pending")
        .body(())
        .unwrap(),
    )
    .await
    .unwrap();
  pending.finish().await.unwrap();
  started.notified().await;
  handle.trigger();
  handle.result().await.unwrap();
  assert_eq!(Arc::strong_count(&marker), 1);
  endpoint.close(0u32.into(), b"test finished");
  driver_task.await.unwrap();
}
