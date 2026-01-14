//! WebSocket example using compio runtime.
//!
//! Run with: cargo run --example websocket-compio --features compio-ws

use std::time::Duration;

use compio_ws::tungstenite::Message;
use tako::Method;
use tako::responder::Responder;
use tako::types::Request;
use tako::ws_compio::CompioWebSocket;
use tako::ws_compio::TakoWsCompio;
use tako::ws_compio::UpgradedStream;

pub async fn ws_echo(req: Request) -> impl Responder {
  TakoWsCompio::new(req, |mut ws: CompioWebSocket<UpgradedStream>| async move {
    if let Err(e) = ws
      .send(Message::Text("Welcome to Tako WS (compio)!".into()))
      .await
    {
      tracing::error!("Failed to send welcome message: {}", e);
      return;
    }

    loop {
      match ws.read().await {
        Ok(Message::Text(txt)) => {
          let response = format!("Echo: {}", txt);
          if let Err(e) = ws.send(Message::Text(response.into())).await {
            tracing::error!("Failed to send echo: {}", e);
            break;
          }
        }
        Ok(Message::Binary(bin)) => {
          if let Err(e) = ws.send(Message::Binary(bin)).await {
            tracing::error!("Failed to send binary: {}", e);
            break;
          }
        }
        Ok(Message::Ping(p)) => {
          if let Err(e) = ws.send(Message::Pong(p)).await {
            tracing::error!("Failed to send pong: {}", e);
            break;
          }
        }
        Ok(Message::Close(_)) => {
          let _ = ws.close(None).await;
          break;
        }
        Ok(_) => {}
        Err(e) => {
          tracing::error!("WebSocket error: {}", e);
          break;
        }
      }
    }
  })
}

pub async fn ws_count(req: Request) -> impl Responder {
  TakoWsCompio::new(req, |mut ws: CompioWebSocket<UpgradedStream>| async move {
    let mut count: u64 = 0;

    loop {
      // Send current count
      let msg = format!("count: {}", count);
      if let Err(e) = ws.send(Message::Text(msg.into())).await {
        tracing::error!("Failed to send count: {}", e);
        break;
      }
      count += 1;

      // Wait 1 second
      compio::time::sleep(Duration::from_secs(1)).await;

      // Check for close message (non-blocking read attempt)
      // For simplicity, we just keep counting until connection drops
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
  tako::serve(listener, router).await;
}
