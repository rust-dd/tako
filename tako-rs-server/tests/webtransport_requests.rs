#![cfg(feature = "webtransport")]

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::CertificateDer;
use tako_rs_core::router::Router;
use tako_rs_server::ServerConfig;
use tako_rs_server::TlsCert;
use tako_rs_server::webtransport::WebTransport;
use tokio_util::sync::CancellationToken;

#[test]
fn server_shutdown_cancels_session_http_requests() {
  run(false);
}

#[test]
fn peer_close_cancels_session_http_requests() {
  run(true);
}

fn run(peer_close: bool) {
  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  #[cfg(not(feature = "compio"))]
  tokio::runtime::Runtime::new()
    .unwrap()
    .block_on(exercise(peer_close));
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new()
    .unwrap()
    .block_on(exercise(peer_close));
}

async fn exercise(peer_close: bool) {
  let identity =
    rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let marker = Arc::new(());
  let handler_marker = marker.clone();
  let started = Arc::new(tokio::sync::Notify::new());
  let handler_started = started.clone();
  let mut router = Router::new();
  router.route(
    http::Method::CONNECT,
    "/session",
    |wt: WebTransport| async {
      wt.on_session(|session| async move { while let Ok(Some(_)) = session.accept_bi().await {} })
    },
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
  #[cfg(feature = "compio")]
  use tako_rs_server::CompioServer as RuntimeServer;
  #[cfg(not(feature = "compio"))]
  use tako_rs_server::Server as RuntimeServer;
  let server = RuntimeServer::builder()
    .tls(TlsCert::der(vec![certificate.clone()], key.into()))
    .config(ServerConfig {
      drain_timeout: Duration::from_millis(300),
      h3_goaway_grace: Duration::from_millis(30),
      ..ServerConfig::default()
    })
    .build()
    .try_spawn_h3("127.0.0.1:0", router)
    .unwrap();
  let address = server.local_addr().unwrap();
  let close_client = CancellationToken::new();
  let client_close = close_client.clone();
  let (done, finished) = tokio::sync::oneshot::channel();
  let client = std::thread::spawn(move || {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
      tokio::time::timeout(
        Duration::from_secs(5),
        send_request(address, certificate, client_close),
      )
      .await
      .unwrap();
    });
    let _ = done.send(());
  });
  timeout(started.notified(), "pending HTTP request never started").await;
  if peer_close {
    close_client.cancel();
    timeout(
      async {
        while Arc::strong_count(&marker) != 2 {
          sleep(Duration::from_millis(5)).await;
        }
      },
      "pending HTTP request survived peer close",
    )
    .await;
  }
  server.shutdown(Duration::from_millis(300)).await;
  close_client.cancel();
  timeout(
    async {
      finished.await.unwrap();
    },
    "client did not finish",
  )
  .await;
  client.join().unwrap();
  assert_eq!(
    Arc::strong_count(&marker),
    1,
    "an HTTP request task outlived its WebTransport connection"
  );
}

async fn send_request(
  address: SocketAddr,
  certificate: CertificateDer<'static>,
  close: CancellationToken,
) {
  let mut roots = rustls::RootCertStore::empty();
  roots.add(certificate).unwrap();
  let mut tls = rustls::ClientConfig::builder()
    .with_root_certificates(roots)
    .with_no_client_auth();
  tls.alpn_protocols = vec![b"h3".to_vec()];
  let config = wtransport::ClientConfig::builder()
    .with_bind_default()
    .with_custom_tls(tls)
    .build();
  let endpoint = wtransport::Endpoint::client(config).unwrap();
  let session = endpoint
    .connect(format!("https://{address}/session"))
    .await
    .unwrap();
  let (mut send, mut recv) = session.quic_connection().open_bi().await.unwrap();
  // Literal QPACK header block: GET https://localhost/pending, with no dynamic references.
  let block = b"\x00\x00\xd1\xd7\x50\x09localhost\x51\x08/pending";
  send.write_all(&[0x01, block.len() as u8]).await.unwrap();
  send.write_all(block).await.unwrap();
  send.finish().unwrap();
  close.cancelled().await;
  session.close(0u32.into(), b"done");
  let _ = recv.read_to_end(1024).await;
  endpoint.wait_idle().await;
}

async fn sleep(duration: Duration) {
  #[cfg(not(feature = "compio"))]
  tokio::time::sleep(duration).await;
  #[cfg(feature = "compio")]
  compio::time::sleep(duration).await;
}

async fn timeout(future: impl std::future::Future<Output = ()>, message: &str) {
  #[cfg(not(feature = "compio"))]
  tokio::time::timeout(Duration::from_secs(2), future)
    .await
    .expect(message);
  #[cfg(feature = "compio")]
  compio::time::timeout(Duration::from_secs(2), future)
    .await
    .expect(message);
}
