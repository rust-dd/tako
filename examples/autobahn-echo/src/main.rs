//! Echo server for the Autobahn `fuzzingclient` suite.
//!
//! Listens on `127.0.0.1:9001` and echoes every text and binary message.

use tako::Method;
use tako::responder::Responder;
use tako::types::Request;
use tako::ws::WebSocketUpgrade;

async fn echo(req: Request) -> impl Responder {
  WebSocketUpgrade::new(req, |mut ws| async move {
    while let Some(Ok(message)) = ws.recv().await {
      if (message.is_text() || message.is_binary()) && ws.send(message).await.is_err() {
        break;
      }
    }
  })
}

#[tokio::main]
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:9001")
    .await
    .expect("bind 127.0.0.1:9001");
  let mut router = tako::router::Router::new();
  router.route(Method::GET, "/", echo);

  println!("autobahn-echo listening on ws://127.0.0.1:9001");
  tako::Server::builder()
    .build()
    .spawn_http(listener, router)
    .result()
    .await
    .expect("HTTP server failed");
}
