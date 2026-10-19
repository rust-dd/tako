#![cfg(all(feature = "ws", feature = "compio"))]

use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::time::Duration;

use tako::CompioServer;
use tako::router::Router;
use tako::types::Request;
use tako::ws::WebSocketUpgrade;

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
  router.get("/done", |req: Request| async move {
    WebSocketUpgrade::new(req, |ws| async move { drop(ws) })
  });
  router.get("/panic", |req: Request| async move {
    WebSocketUpgrade::new(req, |ws| async move {
      let _ws = ws;
      panic!("handler failure under test");
    })
  });
  router.get("/lifetime", |req: Request| async move {
    WebSocketUpgrade::new(req, |ws| async move {
      let _ws = ws;
      std::future::pending::<()>().await;
    })
    .max_lifetime(Duration::from_millis(30))
  });
  router.get("/moved", |req: Request| async move {
    WebSocketUpgrade::new(req, |mut ws| async move {
      compio::runtime::spawn(async move {
        if let Some(Ok(message)) = ws.recv().await {
          ws.send(message).await.unwrap();
        }
      })
      .detach();
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

fn read_exact(stream: &mut TcpStream, len: usize) -> Vec<u8> {
  let mut buf = vec![0; len];
  stream.read_exact(&mut buf).unwrap();
  buf
}

fn connect(address: SocketAddr, path: &str) -> TcpStream {
  let mut stream = TcpStream::connect(address).unwrap();
  stream
    .set_read_timeout(Some(Duration::from_secs(2)))
    .unwrap();
  let upgrade = String::from_utf8(UPGRADE.to_vec())
    .unwrap()
    .replace("/ws", path);
  stream.write_all(upgrade.as_bytes()).unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(read_exact(&mut stream, 1)[0]);
  }
  assert!(
    head.starts_with(b"HTTP/1.1 101"),
    "{}",
    String::from_utf8_lossy(&head)
  );
  stream
}

fn exercise(address: SocketAddr) {
  let mut stream = connect(address, "/ws");
  stream.write_all(&frame(0x81, b"hello")).unwrap();
  assert_eq!(read_exact(&mut stream, 7), b"\x81\x05hello");
  stream.write_all(&frame(0x89, b"p")).unwrap();
  assert_eq!(read_exact(&mut stream, 3), [0x8A, 1, b'p']);
  let batch: Vec<u8> = (0..16u8).flat_map(|i| frame(0x82, &[i])).collect();
  stream.write_all(&batch).unwrap();
  let expected: Vec<u8> = (0..16u8).flat_map(|i| [0x82, 1, i]).collect();
  assert_eq!(read_exact(&mut stream, expected.len()), expected);

  stream
    .write_all(&frame(0x88, &[0x03, 0xE8, b'b', b'y', b'e']))
    .unwrap();
  assert_eq!(
    read_exact(&mut stream, 7),
    [0x88, 5, 0x03, 0xE8, b'b', b'y', b'e']
  );
  let mut rest = Vec::new();
  stream.read_to_end(&mut rest).unwrap();
  assert!(rest.is_empty(), "{rest:?}");
}

fn exercise_lifecycle(address: SocketAddr) {
  for (path, code) in [
    ("/done", 1000u16),
    ("/panic", 1011),
    ("/lifetime", 1001),
    ("/moved", 1000),
  ] {
    let mut stream = connect(address, path);
    if path == "/moved" {
      stream.write_all(&frame(0x81, b"hello")).unwrap();
      assert_eq!(read_exact(&mut stream, 7), b"\x81\x05hello");
    }
    let [high, low] = code.to_be_bytes();
    assert_eq!(read_exact(&mut stream, 4), [0x88, 2, high, low], "{path}");
    let mut rest = Vec::new();
    stream.read_to_end(&mut rest).unwrap();
    assert!(rest.is_empty(), "{path}: {rest:?}");
  }
}

#[test]
fn echoes_on_the_compio_server() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = CompioServer::builder()
      .build()
      .spawn_http(listener, router());
    let client = std::thread::spawn(move || {
      exercise(address);
      exercise_lifecycle(address);
    });
    while !client.is_finished() {
      compio::time::sleep(Duration::from_millis(10)).await;
    }
    client.join().unwrap();
    handle.shutdown(Duration::from_secs(1)).await;
  });
}
