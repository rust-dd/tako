use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use tako::{Method, responder::Responder, types::Request, ws::TakoWs};
use tokio_stream::wrappers::IntervalStream;
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

pub async fn ws_echo(req: Request) -> impl Responder {
    TakoWs::new(req, |mut ws| async move {
        let _ = ws.send(Message::Text("Welcome to Tako WS!".into())).await;

        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Text(txt) => {
                    let _ = ws
                        .send(Message::Text(Utf8Bytes::from(format!("Echo: {txt}"))))
                        .await;
                }
                Message::Binary(bin) => {
                    let _ = ws.send(Message::Binary(bin)).await;
                }
                Message::Ping(p) => {
                    let _ = ws.send(Message::Pong(p)).await;
                }
                Message::Close(_) => {
                    let _ = ws.send(Message::Close(None)).await;
                    break;
                }
                _ => {}
            }
        }
    })
}

pub async fn ws_tick(req: Request) -> impl Responder {
    println!("TICK: {:?}", req);
    TakoWs::new(req, |mut ws| async move {
        let mut ticker =
            IntervalStream::new(tokio::time::interval(Duration::from_secs(1))).enumerate();

        loop {
            tokio::select! {
                msg = ws.next() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }

                Some((i, _)) = ticker.next() => {
                    println!("TICK: {:?}", i);
                    let _ = ws.send(Message::Text(Utf8Bytes::from(format!("tick #{i}")))).await;
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

    tako::serve_tls(listener, router, None, None).await;
}
