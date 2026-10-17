#![cfg(not(feature = "compio"))]

//! An upgraded connection keeps its `max_connections` slot until its socket closes.

use std::future::Future;
use std::time::Duration;

use hyper_util::rt::TokioIo;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;
use tako_rs_server::ServerConfig;
use tokio::io::AsyncRead;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;

const UPGRADE: &[u8] =
  b"GET /upgrade HTTP/1.1\r\nHost: localhost\r\nConnection: upgrade\r\nUpgrade: test\r\n\r\n";
const GET: &[u8] = b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n";

async fn hold_upgrade(req: Request) -> Response {
  if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
    tokio::spawn(async move {
      if let Ok(upgraded) = on_upgrade.await {
        let mut io = TokioIo::new(upgraded);
        let mut buf = [0; 64];
        while matches!(io.read(&mut buf).await, Ok(read) if read > 0) {}
      }
    });
  }
  http::Response::builder()
    .status(http::StatusCode::SWITCHING_PROTOCOLS)
    .header(http::header::CONNECTION, "upgrade")
    .header(http::header::UPGRADE, "test")
    .body(TakoBody::empty())
    .unwrap()
}

fn router() -> Router {
  let mut router = Router::new();
  router.get("/", || async { "ok" });
  router.get("/upgrade", hold_upgrade);
  router
}

fn one_connection() -> ServerConfig {
  ServerConfig {
    max_connections: Some(1),
    ..ServerConfig::default()
  }
}

async fn upgrade<S: AsyncRead + AsyncWrite + Unpin>(mut stream: S) -> S {
  stream.write_all(UPGRADE).await.unwrap();
  let mut head = Vec::new();
  let mut byte = [0; 1];
  while !head.ends_with(b"\r\n\r\n") {
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut byte))
      .await
      .unwrap()
      .unwrap();
    assert_eq!(read, 1, "closed before the upgrade completed");
    head.push(byte[0]);
  }
  assert!(
    head.starts_with(b"HTTP/1.1 101"),
    "{}",
    String::from_utf8_lossy(&head)
  );
  stream
}

async fn assert_upgrade_holds_the_only_slot<C, F, S>(connect: C)
where
  C: Fn() -> F + Send + 'static,
  F: Future<Output = S> + Send,
  S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
  let upgraded = upgrade(connect().await).await;
  let second = tokio::spawn(async move {
    let mut stream = connect().await;
    stream.write_all(GET).await.unwrap();
    let mut byte = [0; 1];
    stream.read_exact(&mut byte).await.unwrap();
  });
  tokio::time::sleep(Duration::from_millis(300)).await;
  assert!(
    !second.is_finished(),
    "served while the upgraded connection held the only slot"
  );
  drop(upgraded);
  tokio::time::timeout(Duration::from_secs(2), second)
    .await
    .expect("not served after the upgraded connection closed")
    .unwrap();
}

#[cfg(unix)]
#[tokio::test]
async fn unix_upgrade_holds_its_connection_slot() {
  let path = std::env::temp_dir().join(format!("tako-upgrade-lease-{}.sock", std::process::id()));
  let _ = std::fs::remove_file(&path);
  let handle = tako_rs_server::Server::builder()
    .config(one_connection())
    .build()
    .try_spawn_unix_http(path.clone(), router())
    .await
    .unwrap();
  let socket = path.clone();
  assert_upgrade_holds_the_only_slot(move || {
    let socket = socket.clone();
    async move { tokio::net::UnixStream::connect(socket).await.unwrap() }
  })
  .await;
  handle.shutdown(Duration::from_secs(1)).await;
  let _ = std::fs::remove_file(&path);
}

#[cfg(feature = "proxy-protocol")]
#[tokio::test]
async fn proxy_protocol_upgrade_holds_its_connection_slot() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = tako_rs_server::Server::builder()
    .config(one_connection())
    .build()
    .spawn_proxy_protocol(listener, router());
  assert_upgrade_holds_the_only_slot(move || async move {
    let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
    stream
      .write_all(b"PROXY TCP4 192.0.2.1 192.0.2.2 40000 80\r\n")
      .await
      .unwrap();
    stream
  })
  .await;
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "tls")]
#[tokio::test]
async fn tls_upgrade_holds_its_connection_slot() {
  use std::sync::Arc;

  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
  let cert = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = tako_rs_server::Server::builder()
    .config(one_connection())
    .tls(tako_rs_server::TlsCert::der(vec![cert.clone()], key.into()))
    .build()
    .spawn_tls(listener, router());
  let mut roots = rustls::RootCertStore::empty();
  roots.add(cert).unwrap();
  let client = rustls::ClientConfig::builder()
    .with_root_certificates(roots)
    .with_no_client_auth();
  let connector = tokio_rustls::TlsConnector::from(Arc::new(client));
  assert_upgrade_holds_the_only_slot(move || {
    let connector = connector.clone();
    async move {
      let tcp = tokio::net::TcpStream::connect(address).await.unwrap();
      let name = rustls::pki_types::ServerName::try_from("localhost").unwrap();
      connector.connect(name, tcp).await.unwrap()
    }
  })
  .await;
  handle.shutdown(Duration::from_secs(1)).await;
}
