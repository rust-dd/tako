#![cfg(feature = "compio")]

use std::future::Future;
use std::time::Duration;

use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_server::CompioServer;
use tako_rs_server::ServerConfig;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio_util::sync::CancellationToken;

fn router(release: CancellationToken) -> Router {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  router.get("/upgrade", move |mut req: Request| {
    let release = release.clone();
    async move {
      let upgraded = hyper::upgrade::on(&mut req);
      compio::runtime::spawn(async move {
        let io = upgraded.await.unwrap();
        release.cancelled().await;
        drop(io);
      })
      .detach();
      http::Response::builder()
        .status(101)
        .header("connection", "upgrade")
        .header("upgrade", "test")
        .body(TakoBody::empty())
        .unwrap()
    }
  });
  router
}

async fn exchange<C, F, S>(connect: C, release: CancellationToken)
where
  C: Fn() -> F + Send + 'static,
  F: Future<Output = S> + Send,
  S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
  let mut upgraded = connect().await;
  upgraded
    .write_all(
      b"GET /upgrade HTTP/1.1\r\nHost: localhost\r\nConnection: upgrade\r\nUpgrade: test\r\n\r\n",
    )
    .await
    .unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(upgraded.read_u8().await.unwrap());
  }
  assert!(head.starts_with(b"HTTP/1.1 101"));
  let second = tokio::spawn(async move {
    let mut second = connect().await;
    second
      .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
      .await
      .unwrap();
    assert_eq!(second.read_u8().await.unwrap(), b'H');
  });
  tokio::time::sleep(Duration::from_millis(150)).await;
  let served_early = second.is_finished();
  release.cancel();
  tokio::time::timeout(Duration::from_secs(2), second)
    .await
    .unwrap()
    .unwrap();
  assert!(
    !served_early,
    "upgrade released the only connection permit before its IO was dropped"
  );
}

async fn check_client<C, F, S>(connect: C, release: CancellationToken)
where
  C: Fn() -> F + Send + 'static,
  F: Future<Output = S> + Send,
  S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
  let (sender, received) = tokio::sync::oneshot::channel();
  let client = std::thread::spawn(move || {
    tokio::runtime::Runtime::new().unwrap().block_on(async {
      tokio::time::timeout(Duration::from_secs(5), exchange(connect, release))
        .await
        .unwrap();
    });
    let _ = sender.send(());
  });
  let result = compio::time::timeout(Duration::from_secs(8), received).await;
  client.join().unwrap();
  result.unwrap().unwrap();
}

fn config() -> ServerConfig {
  ServerConfig {
    max_connections: Some(1),
    ..ServerConfig::default()
  }
}

#[test]
fn upgraded_http_connection_holds_its_slot_until_io_is_dropped() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let release = CancellationToken::new();
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = CompioServer::builder()
      .config(config())
      .build()
      .try_spawn_http(listener, router(release.clone()))
      .unwrap();
    check_client(
      move || async move { tokio::net::TcpStream::connect(address).await.unwrap() },
      release,
    )
    .await;
    server.shutdown(Duration::from_secs(1)).await;
  });
}

#[cfg(feature = "compio-tls")]
#[test]
fn upgraded_tls_connection_holds_its_slot_until_io_is_dropped() {
  use std::sync::Arc;

  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate = identity.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
    let release = CancellationToken::new();
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = CompioServer::builder()
      .config(config())
      .tls(tako_rs_server::TlsCert::der(
        vec![certificate.clone()],
        key.into(),
      ))
      .build()
      .try_spawn_tls(listener, router(release.clone()))
      .unwrap();
    let mut roots = rustls::RootCertStore::empty();
    roots.add(certificate).unwrap();
    let tls = rustls::ClientConfig::builder()
      .with_root_certificates(roots)
      .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(tls));
    check_client(
      move || {
        let connector = connector.clone();
        async move {
          let stream = tokio::net::TcpStream::connect(address).await.unwrap();
          connector
            .connect(
              rustls::pki_types::ServerName::try_from("localhost").unwrap(),
              stream,
            )
            .await
            .unwrap()
        }
      },
      release,
    )
    .await;
    server.shutdown(Duration::from_secs(1)).await;
  });
}
