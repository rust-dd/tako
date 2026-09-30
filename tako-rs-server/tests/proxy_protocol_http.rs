#![cfg(feature = "proxy-protocol")]

use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;

use tako_rs_core::conn_info::ConnInfo;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_server::ServerConfig;
use tako_rs_server::proxy_protocol::ProxyHeader;

const REQUEST: &[u8] =
  b"GET /peer HTTP/1.1\r\nHost: localhost\r\nX-Forwarded-For: 6.6.6.6\r\nConnection: close\r\n\r\n";

fn router() -> Router {
  let mut router = Router::new();
  router.get("/peer", |req: Request| async move {
    let addr = req.extensions().get::<SocketAddr>().unwrap();
    let info = req.extensions().get::<ConnInfo>().unwrap();
    let header = req.extensions().get::<ProxyHeader>().unwrap();
    let forwarded = req.headers().get("forwarded").unwrap().to_str().unwrap();
    let spoofed = req.headers().contains_key("x-forwarded-for");
    format!(
      "addr={addr} peer={:?} version={:?} forwarded={forwarded} spoofed={spoofed}",
      info.peer.as_socket(),
      header.version,
    )
  });
  router
}

fn config() -> ServerConfig {
  ServerConfig {
    proxy_read_timeout: Duration::from_millis(100),
    ..ServerConfig::default()
  }
}

fn v1_preamble() -> Vec<u8> {
  b"PROXY TCP4 203.0.113.7 10.0.0.1 51234 80\r\n".to_vec()
}

fn v2_preamble() -> Vec<u8> {
  let mut preamble = b"\r\n\r\n\0\r\nQUIT\n".to_vec();
  preamble.extend_from_slice(&[0x21, 0x11, 0x00, 0x0C]);
  preamble.extend_from_slice(&[203, 0, 113, 7, 10, 0, 0, 1]);
  preamble.extend_from_slice(&51234u16.to_be_bytes());
  preamble.extend_from_slice(&80u16.to_be_bytes());
  preamble
}

fn exchange(address: SocketAddr, preamble: &[u8], request: &[u8]) -> String {
  let mut stream = TcpStream::connect(address).unwrap();
  stream
    .set_read_timeout(Some(Duration::from_secs(2)))
    .unwrap();
  stream.write_all(preamble).unwrap();
  stream.write_all(request).unwrap();
  let mut response = String::new();
  let _ = stream.read_to_string(&mut response);
  response
}

fn expected(version: &str) -> String {
  format!(
    "addr=203.0.113.7:51234 peer=Some(203.0.113.7:51234) version={version} forwarded=for=\"203.0.113.7:51234\" spoofed=false"
  )
}

fn check_responses(responses: &[String]) {
  assert!(responses[0].ends_with(&expected("V1")), "{}", responses[0]);
  assert!(responses[1].ends_with(&expected("V2")), "{}", responses[1]);
  assert_eq!(
    responses[2], "",
    "a malformed header must drop the connection"
  );
  assert_eq!(
    responses[3], "",
    "a silent client must be dropped at the read deadline"
  );
}

fn run_clients(address: SocketAddr) -> Vec<String> {
  vec![
    exchange(address, &v1_preamble(), REQUEST),
    exchange(address, &v2_preamble(), REQUEST),
    exchange(address, b"PROXY BOGUS\r\n", REQUEST),
    exchange(address, b"", b""),
  ]
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn tokio_proxy_protocol_names_the_real_client() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = tako_rs_server::Server::builder()
    .config(config())
    .build()
    .try_spawn_proxy_protocol(listener, router())
    .unwrap();
  let responses = tokio::task::spawn_blocking(move || run_clients(address))
    .await
    .unwrap();
  check_responses(&responses);
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "compio")]
#[test]
fn compio_proxy_protocol_names_the_real_client() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tako_rs_server::CompioServer::builder()
      .config(config())
      .build()
      .try_spawn_proxy_protocol(listener, router())
      .unwrap();
    let (sender, received) = tokio::sync::oneshot::channel();
    let client = std::thread::spawn(move || sender.send(run_clients(address)).unwrap());
    let responses = compio::time::timeout(Duration::from_secs(10), received)
      .await
      .expect("PROXY clients timed out")
      .unwrap();
    client.join().unwrap();
    check_responses(&responses);
    compio::time::timeout(
      Duration::from_secs(2),
      handle.shutdown(Duration::from_secs(1)),
    )
    .await
    .expect("PROXY server did not shut down");
  });
}
