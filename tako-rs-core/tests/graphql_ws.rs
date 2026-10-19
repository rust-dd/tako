#![cfg(all(feature = "ws", feature = "async-graphql", not(feature = "compio")))]

use std::net::SocketAddr;
use std::time::Duration;

use async_graphql::EmptyMutation;
use async_graphql::Object;
use async_graphql::Schema;
use async_graphql::Subscription;
use futures_util::Stream;
use tako::Server;
use tako_rs_core::graphql::GraphQLSubscription;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpListener;
use tokio::net::TcpStream;
use tokio::time::timeout;

const UPGRADE: &[u8] = b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: graphql-transport-ws\r\n\r\n";
const MASK: [u8; 4] = [5, 6, 7, 8];

struct Query;

#[Object]
impl Query {
  async fn value(&self) -> i32 {
    1
  }
}

struct Ticks;

#[Subscription]
impl Ticks {
  async fn ticks(&self) -> impl Stream<Item = i32> {
    futures_util::stream::iter([1, 2])
  }
}

fn text_frame(payload: &str) -> Vec<u8> {
  let mut frame = vec![0x81, 0x80 | payload.len() as u8];
  frame.extend_from_slice(&MASK);
  frame.extend(
    payload
      .bytes()
      .zip(MASK.iter().cycle())
      .map(|(byte, key)| byte ^ key),
  );
  frame
}

async fn read_text(stream: &mut TcpStream) -> String {
  let mut header = [0; 2];
  timeout(Duration::from_secs(2), stream.read_exact(&mut header))
    .await
    .unwrap()
    .unwrap();
  assert_eq!(header[0], 0x81, "expected a text frame");
  let mut payload = vec![0; usize::from(header[1])];
  stream.read_exact(&mut payload).await.unwrap();
  String::from_utf8(payload).unwrap()
}

async fn connect(address: SocketAddr) -> TcpStream {
  let mut stream = TcpStream::connect(address).await.unwrap();
  stream.write_all(UPGRADE).await.unwrap();
  let mut head = Vec::new();
  while !head.ends_with(b"\r\n\r\n") {
    head.push(stream.read_u8().await.unwrap());
  }
  let head = String::from_utf8(head).unwrap();
  assert!(head.starts_with("HTTP/1.1 101"), "{head}");
  assert!(
    head
      .to_ascii_lowercase()
      .contains("sec-websocket-protocol: graphql-transport-ws"),
    "{head}"
  );
  stream
}

#[tokio::test]
async fn subscriptions_run_over_the_websocket() {
  let schema = Schema::new(Query, EmptyMutation, Ticks);
  let mut router = Router::new();
  router.get("/ws", move |req: Request| {
    let schema = schema.clone();
    async move { GraphQLSubscription::new(req, schema) }
  });
  let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = Server::builder().build().spawn_http(listener, router);

  let mut stream = connect(address).await;
  stream
    .write_all(&text_frame(r#"{"type":"connection_init"}"#))
    .await
    .unwrap();
  assert!(read_text(&mut stream).await.contains("connection_ack"));
  stream
    .write_all(&text_frame(
      r#"{"id":"1","type":"subscribe","payload":{"query":"subscription { ticks }"}}"#,
    ))
    .await
    .unwrap();
  let first = read_text(&mut stream).await;
  let second = read_text(&mut stream).await;
  assert!(first.contains(r#""ticks":1"#), "{first}");
  assert!(second.contains(r#""ticks":2"#), "{second}");
  assert!(read_text(&mut stream).await.contains("complete"));

  handle.shutdown(Duration::from_secs(1)).await;
}
