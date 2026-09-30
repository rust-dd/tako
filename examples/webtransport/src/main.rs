//! W3C WebTransport from a browser, with no certificate to install.
//!
//! Run it and open <http://127.0.0.1:3000> in Chrome, Edge, or Firefox. The
//! page opens a session to `https://127.0.0.1:4433/echo` with
//! `serverCertificateHashes`, which accepts the short-lived self-signed
//! certificate generated at startup, then echoes a stream and a datagram.

use rcgen::CertificateParams;
use rcgen::KeyPair;
use sha2::Digest;
use sha2::Sha256;
use tako::Method;
use tako::TlsCert;
use tako::body::TakoBody;
use tako::router::Router;
use tako::types::BoxError;
use tako::types::Response;
use tako::webtransport::WebTransport;
use tako::webtransport::WebTransportSession;
use time::Duration;
use time::OffsetDateTime;
use tokio::io::AsyncWriteExt;

async fn echo(wt: WebTransport) -> Response {
  wt.on_session(|session: WebTransportSession| async move {
    println!("session from {}", session.remote_address());

    let datagrams = tokio::spawn({
      let session = session.clone();
      async move {
        while let Ok(Some(datagram)) = session.read_datagram().await {
          let _ = session.send_datagram(datagram);
        }
      }
    });
    while let Ok(Some(stream)) = session.accept_bi().await {
      tokio::spawn(async move {
        let (mut send, mut recv) = stream.split();
        let _ = tokio::io::copy(&mut recv, &mut send).await;
        let _ = send.shutdown().await;
      });
    }
    datagrams.abort();

    let close = session.closed().await;
    println!("session closed: code {} ({:?})", close.code, close.reason);
  })
}

/// A self-signed ECDSA P-256 certificate valid for ten days: the kind
/// `serverCertificateHashes` accepts. Returns it with its SHA-256 hash.
fn certificate() -> Result<(TlsCert, Vec<u8>), BoxError> {
  let key = KeyPair::generate()?;
  let mut params = CertificateParams::new(vec!["localhost".into(), "127.0.0.1".into()])?;
  params.not_before = OffsetDateTime::now_utc() - Duration::hours(1);
  params.not_after = OffsetDateTime::now_utc() + Duration::days(10);
  let cert = params.self_signed(&key)?;
  let hash = Sha256::digest(cert.der()).to_vec();
  Ok((TlsCert::der(vec![cert.der().clone()], key.into()), hash))
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
  let (tls, hash) = certificate()?;
  let hash: Vec<String> = hash.iter().map(u8::to_string).collect();
  let page = include_str!("index.html").replace("CERT_HASH", &hash.join(","));

  let mut pages = Router::new();
  pages.get("/", move || {
    let mut page = Response::new(TakoBody::from(page.clone()));
    page.headers_mut().insert(
      tako::header::CONTENT_TYPE,
      tako::header::HeaderValue::from_static("text/html; charset=utf-8"),
    );
    async move { page }
  });
  let mut sessions = Router::new();
  sessions.route(Method::CONNECT, "/echo", echo);

  // Browsers load pages over HTTP/3 only from a trusted certificate, so the
  // page comes over HTTP/1.1; `127.0.0.1` still counts as a secure context.
  let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await?;
  let page_server = tako::Server::builder()
    .build()
    .try_spawn_http(listener, pages)?;
  let webtransport = tako::Server::builder()
    .tls(tls)
    .build()
    .try_spawn_h3("127.0.0.1:4433", sessions)?;

  println!("open http://127.0.0.1:3000");
  tokio::select! {
    result = page_server.result() => result?,
    result = webtransport.result() => result?,
  }
  Ok(())
}
