//! Plain echo server used by the Autobahn `fuzzingclient` test suite.
//!
//! Listens on `127.0.0.1:9001`. Every text and binary frame is echoed back
//! verbatim; ping is replied to with pong; close is mirrored before the
//! task exits. No subprotocol, no compression, no auth — the suite varies
//! all those itself.

use futures_util::SinkExt;
use futures_util::StreamExt;
use tako::Method;
use tako::responder::Responder;
use tako::types::Request;
use tako::ws::TakoWs;
use tokio_tungstenite::tungstenite::Message;

async fn echo(req: Request) -> impl Responder {
  TakoWs::new(req, |mut ws| async move {
    while let Some(Ok(msg)) = ws.next().await {
      match msg {
        Message::Text(t) => {
          let _ = ws.send(Message::Text(t)).await;
        }
        Message::Binary(b) => {
          let _ = ws.send(Message::Binary(b)).await;
        }
        Message::Ping(p) => {
          let _ = ws.send(Message::Pong(p)).await;
        }
        Message::Close(c) => {
          let _ = ws.send(Message::Close(c)).await;
          break;
        }
        _ => {}
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
  tako::serve(listener, router).await;
}
