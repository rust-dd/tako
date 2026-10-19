#![cfg(all(feature = "ws", not(feature = "compio")))]

use std::net::SocketAddr;
use std::time::Duration;

use tako::Server;
use tako::router::Router;
use tako::types::Request;
use tako::ws::WebSocketUpgrade;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::time::timeout;

const UPGRADE: &[u8] = b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\r\n";
const MASK: [u8; 4] = [9, 8, 7, 6];

fn router() -> Router {
  let mut router = Router::new();
  router.get("/ws", |req: Request| async move {
    WebSocketUpgrade::new(req, |mut ws| async move {
      while let Some(Ok(message)) = ws.recv().await {
        if (message.is_text() || message.is_binary()) && ws.send(message).await.is_err() {
          break;
        }
      }
    })
  });
  router
}

/// A masked client frame with a payload shorter than 126 bytes.
fn frame(first: u8, payload: &[u8]) -> Vec<u8> {
  let mut frame = vec![first, 0x80 | payload.len() as u8];
  frame.extend_from_slice(&MASK);
  frame.extend(
    payload
      .iter()
      .zip(MASK.iter().cycle())
      .map(|(byte, key)| byte ^ key),
  );
  frame
}

async fn read_exact(stream: &mut TcpStream, len: usize) -> Vec<u8> {
  let mut buf = vec![0; len];
  timeout(Duration::from_secs(2), stream.read_exact(&mut buf))
    .await
    .unwrap()
    .unwrap();
  buf
}

async fn connect(address: SocketAddr) -> TcpStream {
  let mut stream = TcpStream::connect(address).await.unwrap();
  stream.write_all(UPGRADE).await.unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(read_exact(&mut stream, 1).await[0]);
  }
  assert!(
    head.starts_with(b"HTTP/1.1 101"),
    "{}",
    String::from_utf8_lossy(&head)
  );
  stream
}

async fn exercise(address: SocketAddr) {
  let mut stream = connect(address).await;
  stream.write_all(&frame(0x81, b"hello")).await.unwrap();
  assert_eq!(read_exact(&mut stream, 7).await, b"\x81\x05hello");
  stream.write_all(&frame(0x82, &[1, 2, 3])).await.unwrap();
  assert_eq!(read_exact(&mut stream, 5).await, [0x82, 3, 1, 2, 3]);
  stream.write_all(&frame(0x89, b"p")).await.unwrap();
  assert_eq!(read_exact(&mut stream, 3).await, [0x8A, 1, b'p']);

  let batch: Vec<u8> = (0..16u8).flat_map(|i| frame(0x82, &[i])).collect();
  stream.write_all(&batch).await.unwrap();
  let expected: Vec<u8> = (0..16u8).flat_map(|i| [0x82, 1, i]).collect();
  assert_eq!(read_exact(&mut stream, expected.len()).await, expected);

  stream
    .write_all(&frame(0x88, &[0x03, 0xE8, b'b', b'y', b'e']))
    .await
    .unwrap();
  assert_eq!(
    read_exact(&mut stream, 7).await,
    [0x88, 5, 0x03, 0xE8, b'b', b'y', b'e']
  );
  let mut rest = Vec::new();
  timeout(Duration::from_secs(2), stream.read_to_end(&mut rest))
    .await
    .unwrap()
    .unwrap();
  assert!(rest.is_empty(), "{rest:?}");
}

#[tokio::test]
async fn echoes_on_the_multi_thread_server() {
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = Server::builder().build().spawn_http(listener, router());
  exercise(address).await;
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(all(unix, feature = "per-thread"))]
#[tokio::test]
async fn echoes_on_the_per_thread_server() {
  let address = std::net::TcpListener::bind("127.0.0.1:0")
    .unwrap()
    .local_addr()
    .unwrap();
  let (threads, shutdown) = tako::spawn_per_thread(
    &address.to_string(),
    router(),
    tako::PerThreadConfig {
      workers: 2,
      pin_to_core: false,
      ..tako::PerThreadConfig::default()
    },
  )
  .unwrap();
  timeout(Duration::from_secs(2), shutdown.wait_for_bind_outcome(2))
    .await
    .unwrap()
    .unwrap();
  exercise(address).await;
  shutdown.trigger();
  tokio::task::spawn_blocking(move || threads.into_iter().for_each(|t| t.join().unwrap()))
    .await
    .unwrap();
}
