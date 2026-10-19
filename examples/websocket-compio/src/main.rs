//! WebSocket echo and counter on the compio runtime.
//!
//! Run with: cargo run --manifest-path examples/websocket-compio/Cargo.toml

use std::time::Duration;

use tako::Method;
use tako::responder::Responder;
use tako::types::Request;
use tako::ws::Message;
use tako::ws::WebSocketUpgrade;

pub async fn ws_echo(req: Request) -> impl Responder {
  WebSocketUpgrade::new(req, |mut ws| async move {
    if ws
      .send(Message::text("Welcome to Tako WS (compio)!"))
      .await
      .is_err()
    {
      return;
    }
    while let Some(Ok(message)) = ws.recv().await {
      let reply = match message {
        Message::Text(text) => Message::text(format!("Echo: {text}")),
        Message::Binary(data) => Message::Binary(data),
        _ => continue,
      };
      if let Err(error) = ws.send(reply).await {
        tracing::error!("Failed to send echo: {error}");
        break;
      }
    }
  })
}

pub async fn ws_count(req: Request) -> impl Responder {
  WebSocketUpgrade::new(req, |mut ws| async move {
    for count in 0u64.. {
      if ws.send(Message::text(format!("count: {count}"))).await.is_err() {
        break;
      }
      compio::time::sleep(Duration::from_secs(1)).await;
    }
  })
}

#[compio::main]
async fn main() {
  let listener = compio::net::TcpListener::bind("127.0.0.1:8080")
    .await
    .unwrap();
  let mut router = tako::router::Router::new();

  router.route(Method::GET, "/ws/echo", ws_echo);
  router.route(Method::GET, "/ws/count", ws_count);

  println!("WebSocket server running at:");
  println!("  - ws://127.0.0.1:8080/ws/echo");
  println!("  - ws://127.0.0.1:8080/ws/count");
  tako::CompioServer::builder()
    .build()
    .spawn_http(listener, router)
    .result()
    .await
    .expect("HTTP server failed");
}
