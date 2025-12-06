use std::time::Duration;

use compio::net::TcpListener;
use compio::ws::tungstenite::Error;
use tako::Method;
use tako::router::Router;

#[compio::main]
async fn main() {
  let listener = TcpListener::bind("127.0.0.1:8080").await.unwrap();

  let mut router = Router::new();

  router.route(Method::GET, "/ws/tick", move |req| async move {
    tako::ws_compio::TakoWs::new(
      req,
      |mut ws| async move {
        let mut interval = compio::time::interval(Duration::from_secs(1));
        let mut count = 0u64;

        loop {
          match ws.read().await {
            Ok(msg) => {
              if msg.is_text() || msg.is_binary() {
                ws.send(msg).await.unwrap();
              }
            }
            Err(e) => match e {
              Error::ConnectionClosed => {
                tracing::info!("Connection closed normally");
                break;
              }
              _ => {
                tracing::error!("Error: {e}");
                return;
              }
            },
          }
        }
      },
      listener.accept().await.unwrap().0.clone(),
    )
  });

  tako::serve(listener, router).await;
}
