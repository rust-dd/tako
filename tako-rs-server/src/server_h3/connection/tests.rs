use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use tako_rs_core::router::Router;
use tokio_util::sync::CancellationToken;

use super::handle_connection;

#[tokio::test]
async fn concurrent_streams_do_not_contend_on_the_shared_router_refcount() {
  tokio::time::timeout(Duration::from_secs(5), exercise_router_sharing())
    .await
    .expect("HTTP/3 router sharing timed out");
}

async fn exercise_router_sharing() {
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let mut tls = rustls::ServerConfig::builder()
    .with_no_client_auth()
    .with_single_cert(vec![certificate.clone()], key.into())
    .unwrap();
  tls.alpn_protocols = vec![b"h3".to_vec()];
  let server = crate::server_h3::run::bind_endpoint(
    "127.0.0.1:0",
    Arc::new(tls),
    &crate::ServerConfig::default(),
    #[cfg(feature = "http3-qlog")]
    None,
  )
  .unwrap()
  .endpoint;
  let address = server.local_addr().unwrap();

  let (started, mut requests_started) = tokio::sync::mpsc::unbounded_channel();
  let release = CancellationToken::new();
  let handler_release = release.clone();
  let mut router = Router::new();
  router.get("/pending", move || {
    let started = started.clone();
    let release = handler_release.clone();
    async move {
      started.send(()).unwrap();
      release.cancelled().await;
      Bytes::from_static(b"done")
    }
  });
  let router = Arc::new(router);
  let connection_router = router.clone();
  let shutdown = CancellationToken::new();
  let connection_shutdown = shutdown.clone();
  let connection_task = tokio::spawn(async move {
    let connection = server.accept().await.unwrap().await.unwrap();
    let remote_addr = connection.remote_address();
    handle_connection(
      connection,
      connection_router,
      remote_addr,
      connection_shutdown,
      Duration::from_secs(1),
    )
    .await
    .unwrap();
  });

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
    .connect(address, "localhost")
    .unwrap()
    .await
    .unwrap();
  let (mut driver, mut client) = h3::client::new(h3_quinn::Connection::new(connection))
    .await
    .unwrap();
  let driver_task =
    tokio::spawn(async move { std::future::poll_fn(|cx| driver.poll_close(cx)).await });

  let mut requests = Vec::new();
  for _ in 0..8 {
    let mut request = client
      .send_request(
        http::Request::builder()
          .uri("https://localhost/pending")
          .body(())
          .unwrap(),
      )
      .await
      .unwrap();
    request.finish().await.unwrap();
    requests.push(request);
  }
  for _ in &requests {
    requests_started.recv().await.unwrap();
  }
  assert_eq!(
    Arc::strong_count(&router),
    2,
    "all request tasks should share the connection's one global router reference"
  );

  release.cancel();
  for mut request in requests {
    assert_eq!(
      request.recv_response().await.unwrap().status(),
      http::StatusCode::OK
    );
    while request.recv_data().await.unwrap().is_some() {}
  }
  shutdown.cancel();
  connection_task.await.unwrap();
  assert_eq!(Arc::strong_count(&router), 1);
  endpoint.close(0u32.into(), b"test finished");
  driver_task.await.unwrap();
}
