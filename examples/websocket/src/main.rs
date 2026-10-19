use std::time::Duration;

use tako::Method;
use tako::responder::Responder;
use tako::types::Request;
use tako::ws::Message;
use tako::ws::WebSocketUpgrade;

pub async fn ws_echo(req: Request) -> impl Responder {
  WebSocketUpgrade::new(req, |mut ws| async move {
    if ws.send(Message::text("Welcome to Tako WS!")).await.is_err() {
      return;
    }
    while let Some(Ok(message)) = ws.recv().await {
      let reply = match message {
        Message::Text(text) => Message::text(format!("Echo: {text}")),
        Message::Binary(data) => Message::Binary(data),
        _ => continue,
      };
      if ws.send(reply).await.is_err() {
        break;
      }
    }
  })
}

pub async fn ws_tick(req: Request) -> impl Responder {
  WebSocketUpgrade::new(req, |mut ws| async move {
    let mut ticker = tokio::time::interval(Duration::from_secs(1));
    let mut tick = 0u64;
    loop {
      tokio::select! {
        message = ws.recv() => match message {
          Some(Ok(_)) => {}
          _ => break,
        },
        _ = ticker.tick() => {
          if ws.send(Message::text(format!("tick #{tick}"))).await.is_err() {
            break;
          }
          tick += 1;
        }
      }
    }
  })
}

#[tokio::main]
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080")
    .await
    .unwrap();
  let mut router = tako::router::Router::new();

  router.route(Method::GET, "/ws/echo", ws_echo);
  router.route(Method::GET, "/ws/tick", ws_tick);

  tako::Server::builder()
    .build()
    .spawn_http(listener, router)
    .result()
    .await
    .expect("HTTP server failed");
}
