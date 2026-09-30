#![cfg(all(feature = "compio", feature = "http3"))]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Buf;
use bytes::Bytes;
use rustls::pki_types::CertificateDer;
use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_server::CompioServer;
use tako_rs_server::ServerConfig;
use tako_rs_server::TlsCert;

struct Exchange {
  echo_status: http::StatusCode,
  echo_body: Bytes,
  echo_trailer: String,
  transport: String,
}

#[test]
fn compio_h3_streams_data_and_trailers_and_cancels_pending_requests() {
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  compio::runtime::Runtime::new().unwrap().block_on(async {
    compio::time::timeout(Duration::from_secs(10), exercise_h3())
      .await
      .expect("compio HTTP/3 lifecycle timed out");
  });
}

async fn exercise_h3() {
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
  router.get("/transport", |request: Request| async move {
    format!(
      "{:?}",
      request.extensions().get::<ConnInfo>().unwrap().transport
    )
  });
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
  let handle = CompioServer::builder()
    .tls(TlsCert::der(vec![certificate.clone()], key.into()))
    .config(ServerConfig {
      drain_timeout: Duration::from_millis(100),
      h3_goaway_grace: Duration::from_millis(20),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_h3("127.0.0.1:0", router)
    .unwrap();
  let address = handle.local_addr().unwrap();

  let (sender, received) = tokio::sync::oneshot::channel();
  let client = std::thread::spawn(move || run_client(address, certificate, sender));
  let exchange = compio::time::timeout(Duration::from_secs(5), received)
    .await
    .expect("HTTP/3 client timed out")
    .unwrap();
  assert_eq!(exchange.echo_status, http::StatusCode::OK);
  assert_eq!(exchange.echo_body, "first");
  assert_eq!(exchange.echo_trailer, "verified");
  assert_eq!(exchange.transport, "Http3");

  started.notified().await;
  handle.trigger();
  handle.result().await.unwrap();
  assert_eq!(Arc::strong_count(&marker), 1);
  client.join().unwrap();
}

fn run_client(
  address: SocketAddr,
  certificate: CertificateDer<'static>,
  sender: tokio::sync::oneshot::Sender<Exchange>,
) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
  runtime.block_on(async move {
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
    let echo_status = echo.recv_response().await.unwrap().status();
    let mut first = echo.recv_data().await.unwrap().unwrap();
    let echo_body = first.copy_to_bytes(first.remaining());
    let mut trailers = http::HeaderMap::new();
    trailers.insert("x-checksum", "verified".parse().unwrap());
    echo.send_trailers(trailers).await.unwrap();
    echo.finish().await.unwrap();
    assert!(echo.recv_data().await.unwrap().is_none());
    let echo_trailer = echo.recv_trailers().await.unwrap().unwrap()["x-checksum"]
      .to_str()
      .unwrap()
      .to_owned();

    let mut transport = client
      .send_request(
        http::Request::builder()
          .uri("https://localhost/transport")
          .body(())
          .unwrap(),
      )
      .await
      .unwrap();
    transport.finish().await.unwrap();
    transport.recv_response().await.unwrap();
    let mut body = transport.recv_data().await.unwrap().unwrap();
    let transport = String::from_utf8(body.copy_to_bytes(body.remaining()).to_vec()).unwrap();

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

    let _ = sender.send(Exchange {
      echo_status,
      echo_body,
      echo_trailer,
      transport,
    });
    let _ = driver_task.await;
    endpoint.close(0u32.into(), b"test finished");
  });
}
