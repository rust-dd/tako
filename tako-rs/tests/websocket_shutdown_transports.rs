#![cfg(all(
  feature = "ws",
  any(
    unix,
    feature = "proxy-protocol",
    all(feature = "tls", not(feature = "compio")),
    feature = "compio-tls"
  )
))]

use std::future::Future;
use std::io::Read;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

#[cfg(feature = "compio")]
use tako::CompioServer as Server;
#[cfg(not(feature = "compio"))]
use tako::Server;
use tako::ServerHandle;
use tako::router::Router;
use tako::types::Request;
use tako::ws::WebSocketUpgrade;
use tokio::sync::mpsc;
use tokio::sync::oneshot;

const UPGRADE: &[u8] = b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\r\n";
const DEADLINE: Duration = Duration::from_secs(3);

struct SessionDrop(Arc<AtomicBool>);

impl Drop for SessionDrop {
  fn drop(&mut self) {
    self.0.store(true, Ordering::Release);
  }
}

fn endpoint() -> (Router, mpsc::Receiver<()>, Arc<AtomicBool>) {
  let (started, received) = mpsc::channel(1);
  let dropped = Arc::new(AtomicBool::new(false));
  let probe = dropped.clone();
  let mut router = Router::new();
  router.get("/ws", move |request: Request| {
    let started = started.clone();
    let dropped = probe.clone();
    async move {
      WebSocketUpgrade::new(request, move |mut socket| async move {
        let _probe = SessionDrop(dropped);
        started.send(()).await.unwrap();
        while let Some(Ok(_)) = socket.recv().await {}
      })
    }
  });

  (router, received, dropped)
}

async fn within<F: Future>(future: F) -> F::Output {
  #[cfg(not(feature = "compio"))]
  let result = tokio::time::timeout(DEADLINE, future).await;
  #[cfg(feature = "compio")]
  let result = compio::time::timeout(DEADLINE, future).await;
  result.expect("WebSocket shutdown did not finish within the deadline")
}

async fn check_shutdown<C, S>(
  server: ServerHandle,
  mut started: mpsc::Receiver<()>,
  dropped: Arc<AtomicBool>,
  connect: C,
) where
  C: FnOnce() -> std::io::Result<S> + Send + 'static,
  S: Read + Write,
{
  let (sent, received) = oneshot::channel();
  let client = std::thread::spawn(move || {
    let result = (|| {
      let mut stream = connect()?;
      stream.write_all(UPGRADE)?;
      stream.flush()?;
      let mut head = Vec::new();
      while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
        assert!(head.len() < 8192);
      }
      assert!(head.starts_with(b"HTTP/1.1 101"), "{head:?}");

      let mut frames = Vec::new();
      stream.read_to_end(&mut frames)?;
      Ok::<_, std::io::Error>(frames)
    })();
    let _ = sent.send(result);
  });
  within(started.recv()).await.unwrap();

  within(server.shutdown(Duration::from_secs(1))).await;
  assert!(
    dropped.load(Ordering::Acquire),
    "shutdown returned before the WebSocket handler was dropped"
  );
  let result = within(received).await;
  client.join().unwrap();
  assert_eq!(result.unwrap().unwrap(), [0x88, 2, 0x03, 0xE9]);
}

#[cfg(any(
  feature = "proxy-protocol",
  all(feature = "tls", not(feature = "compio")),
  feature = "compio-tls"
))]
fn connect_tcp(address: std::net::SocketAddr) -> std::io::Result<std::net::TcpStream> {
  let stream = std::net::TcpStream::connect_timeout(&address, DEADLINE)?;
  stream.set_read_timeout(Some(DEADLINE))?;
  stream.set_write_timeout(Some(DEADLINE))?;
  Ok(stream)
}

#[cfg(unix)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn unix_websocket_shutdown_sends_going_away_and_drops_the_handler() {
  let path = std::env::temp_dir().join(format!("tako-ws-stop-{}.sock", std::process::id()));
  let (router, started, dropped) = endpoint();
  let server = Server::builder()
    .build()
    .try_spawn_unix_http(path.clone(), router)
    .await
    .unwrap();

  check_shutdown(server, started, dropped, move || {
    let stream = std::os::unix::net::UnixStream::connect(path)?;
    stream.set_read_timeout(Some(DEADLINE))?;
    stream.set_write_timeout(Some(DEADLINE))?;
    Ok(stream)
  })
  .await;
}

#[cfg(feature = "proxy-protocol")]
#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn proxy_websocket_shutdown_sends_going_away_and_drops_the_handler() {
  #[cfg(not(feature = "compio"))]
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  #[cfg(feature = "compio")]
  let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let (router, started, dropped) = endpoint();
  let server = Server::builder()
    .build()
    .try_spawn_proxy_protocol(listener, router)
    .unwrap();

  check_shutdown(server, started, dropped, move || {
    let mut stream = connect_tcp(address)?;
    stream.write_all(b"PROXY TCP4 192.0.2.1 192.0.2.2 40000 80\r\n")?;
    Ok(stream)
  })
  .await;
}

#[cfg(any(all(feature = "tls", not(feature = "compio")), feature = "compio-tls"))]
#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn tls_websocket_shutdown_sends_going_away_and_drops_the_handler() {
  use compio::tls::rustls;

  let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
  let certificate = rustls::pki_types::CertificateDer::from(
    include_bytes!("fixtures/websocket-shutdown-cert.der").to_vec(),
  );
  let key = rustls::pki_types::PrivatePkcs8KeyDer::from(
    include_bytes!("fixtures/websocket-shutdown-key.der").to_vec(),
  );
  #[cfg(not(feature = "compio"))]
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  #[cfg(feature = "compio")]
  let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let (router, started, dropped) = endpoint();
  let server = Server::builder()
    .tls(tako::TlsCert::der(vec![certificate.clone()], key.into()))
    .build()
    .try_spawn_tls(listener, router)
    .unwrap();

  let mut roots = rustls::RootCertStore::empty();
  roots.add(certificate).unwrap();
  let mut config = rustls::ClientConfig::builder()
    .with_root_certificates(roots)
    .with_no_client_auth();
  config.alpn_protocols = vec![b"http/1.1".to_vec()];

  check_shutdown(server, started, dropped, move || {
    let connection = rustls::ClientConnection::new(
      Arc::new(config),
      rustls::pki_types::ServerName::try_from("localhost").unwrap(),
    )
    .map_err(std::io::Error::other)?;
    Ok(rustls::StreamOwned::new(connection, connect_tcp(address)?))
  })
  .await;
}
