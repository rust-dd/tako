#![cfg(feature = "webtransport")]

//! Drives W3C WebTransport sessions through the `wtransport` client.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use http::Method;
use http::StatusCode;
use rustls::pki_types::CertificateDer;
use tako_rs_core::router::Router;
use tako_rs_core::types::Response;
use tako_rs_server::TlsCert;
use tako_rs_server::webtransport::WebTransport;
use tako_rs_server::webtransport::WebTransportClose;
use tako_rs_server::webtransport::WebTransportSession;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

fn identity() -> (CertificateDer<'static>, TlsCert) {
  let identity =
    rcgen::generate_simple_self_signed(vec!["localhost".into(), "127.0.0.1".into()]).unwrap();
  let certificate = identity.cert.der().clone();
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
  let tls = TlsCert::der(vec![certificate.clone()], key.into());
  (certificate, tls)
}

async fn echo(wt: WebTransport) -> Response {
  wt.on_session(serve_session)
}

/// Answers the client script step by step, echoes datagrams, and closes the
/// session once the client asks for it on a new stream.
async fn serve_session(session: WebTransportSession) {
  let mut greeting = session.open_uni().await.unwrap();
  greeting.write_all(b"welcome").await.unwrap();
  greeting.shutdown().await.unwrap();

  let (mut send, mut recv) = session.accept_bi().await.unwrap().unwrap().split();
  let mut data = Vec::new();
  recv.read_to_end(&mut data).await.unwrap();
  send.write_all(&data).await.unwrap();
  send.shutdown().await.unwrap();

  let mut note = String::new();
  let mut uni = session.accept_uni().await.unwrap().unwrap();
  uni.read_to_string(&mut note).await.unwrap();
  let mut reply = session.open_bi().await.unwrap();
  reply
    .write_all(format!("noted: {note}").as_bytes())
    .await
    .unwrap();
  reply.shutdown().await.unwrap();

  loop {
    let datagram = std::pin::pin!(session.read_datagram());
    let stream = std::pin::pin!(session.accept_bi());
    match futures_util::future::select(datagram, stream).await {
      futures_util::future::Either::Left((Ok(Some(datagram)), _)) => {
        let _ = session.send_datagram(datagram);
      }
      futures_util::future::Either::Right((Ok(Some(_)), _)) => {
        session.close(7, "done").await.unwrap();
        break;
      }
      _ => break,
    }
  }
}

fn router(closes: tokio::sync::mpsc::UnboundedSender<WebTransportClose>) -> Router {
  let mut router = Router::new();
  router.route(Method::CONNECT, "/echo", echo);
  router.route(Method::CONNECT, "/linger", move |wt: WebTransport| {
    let closes = closes.clone();
    async move {
      wt.on_session(move |session: WebTransportSession| async move {
        let _ = closes.send(session.closed().await);
      })
    }
  });
  router.route(Method::CONNECT, "/private", |_: WebTransport| async {
    StatusCode::UNAUTHORIZED
  });
  router
}

async fn read_all(mut stream: impl tokio::io::AsyncRead + Unpin) -> String {
  let mut text = String::new();
  stream.read_to_string(&mut text).await.unwrap();
  text
}

async fn exercise(
  address: SocketAddr,
  certificate: CertificateDer<'static>,
  mut closes: tokio::sync::mpsc::UnboundedReceiver<WebTransportClose>,
) {
  let mut roots = rustls::RootCertStore::empty();
  roots.add(certificate).unwrap();
  let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
  let mut tls = rustls::ClientConfig::builder_with_provider(provider)
    .with_protocol_versions(&[&rustls::version::TLS13])
    .unwrap()
    .with_root_certificates(roots)
    .with_no_client_auth();
  tls.alpn_protocols = vec![b"h3".to_vec()];
  let config = wtransport::ClientConfig::builder()
    .with_bind_default()
    .with_custom_tls(tls)
    .build();
  let endpoint = wtransport::Endpoint::client(config).unwrap();
  let url = |path: &str| format!("https://{address}{path}");

  let connection = endpoint.connect(url("/echo")).await.unwrap();

  let greeting = connection.accept_uni().await.unwrap();
  assert_eq!(read_all(greeting).await, "welcome");

  let (mut send, recv) = connection.open_bi().await.unwrap().await.unwrap();
  send.write_all(b"ping").await.unwrap();
  send.finish().await.unwrap();
  assert_eq!(read_all(recv).await, "ping");

  let mut note = connection.open_uni().await.unwrap().await.unwrap();
  note.write_all(b"hello").await.unwrap();
  note.finish().await.unwrap();
  let (_, reply) = connection.accept_bi().await.unwrap();
  assert_eq!(read_all(reply).await, "noted: hello");

  // Datagrams are unreliable even on loopback, so retry until one echoes.
  let echoed = tokio::time::timeout(Duration::from_secs(5), async {
    loop {
      connection.send_datagram(b"datagram").unwrap();
      if let Ok(Ok(datagram)) =
        tokio::time::timeout(Duration::from_millis(200), connection.receive_datagram()).await
      {
        return datagram.payload();
      }
    }
  })
  .await
  .expect("no datagram came back");
  assert_eq!(echoed, Bytes::from_static(b"datagram"));

  // A new stream asks the server to close the session with a code.
  let (mut ask, _) = connection.open_bi().await.unwrap().await.unwrap();
  ask.write_all(b"close").await.unwrap();
  match connection.accept_bi().await {
    Err(wtransport::error::ConnectionError::ApplicationClosed(close)) => {
      assert_eq!(close.code(), 7u32.into());
      assert_eq!(close.reason(), b"done");
    }
    other => panic!("expected the server's close, got {other:?}"),
  }

  // Closing the connection ends a session the server is only watching.
  let lingering = endpoint.connect(url("/linger")).await.unwrap();
  lingering.close(0u32.into(), b"bye");
  let close = tokio::time::timeout(Duration::from_secs(2), closes.recv())
    .await
    .expect("the server did not notice the closed session");
  assert_eq!(close, Some(WebTransportClose::default()));

  for path in ["/private", "/missing"] {
    let rejected = endpoint.connect(url(path)).await.err();
    assert!(
      matches!(
        rejected,
        Some(wtransport::error::ConnectingError::SessionRejected)
      ),
      "{path}: {rejected:?}"
    );
  }
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn wtransport_client_uses_streams_and_datagrams_on_tokio() {
  let (certificate, tls) = identity();
  let (closes, closed) = tokio::sync::mpsc::unbounded_channel();
  let handle = tako_rs_server::Server::builder()
    .tls(tls)
    .build()
    .try_spawn_h3("127.0.0.1:0", router(closes))
    .unwrap();
  let address = handle.local_addr().unwrap();
  tokio::time::timeout(
    Duration::from_secs(20),
    exercise(address, certificate, closed),
  )
  .await
  .expect("WebTransport client timed out");
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "compio")]
#[test]
fn wtransport_client_uses_streams_and_datagrams_on_compio() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let (certificate, tls) = identity();
    let (closes, closed) = tokio::sync::mpsc::unbounded_channel();
    let handle = tako_rs_server::CompioServer::builder()
      .tls(tls)
      .build()
      .try_spawn_h3("127.0.0.1:0", router(closes))
      .unwrap();
    let address = handle.local_addr().unwrap();
    let (done, finished) = tokio::sync::oneshot::channel();
    let client = std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
      runtime.block_on(exercise(address, certificate, closed));
      let _ = done.send(());
    });
    if compio::time::timeout(Duration::from_secs(20), finished)
      .await
      .is_err()
    {
      panic!("WebTransport client timed out");
    }
    if let Err(panic) = client.join() {
      std::panic::resume_unwind(panic);
    }
    handle.shutdown(Duration::from_secs(1)).await;
  });
}
