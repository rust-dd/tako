use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream;
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;
use tako::sse::Sse;

async fn ticker() -> impl Responder {
  let s = stream::unfold(0u64, |i| async move {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    let msg = format!("tick: {i}");
    Some((Bytes::from(msg), i + 1))
  });

  Sse::new(s)
}

async fn counter() -> impl Responder {
  let s = stream::iter(0u8..=5).then(|n| async move {
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Bytes::from(format!("count: {n}"))
  });

  Sse::new(s)
}

#[tokio::main]
async fn main() -> Result<()> {
  let mut router = Router::new();
  router.route(Method::GET, "/events", ticker);
  router.route(Method::GET, "/count", counter);

  println!("Starting HTTP/3 SSE server on [::]:4433");
  println!("Test with the client: cargo run --bin client [::1]:4433 /count");
  println!("Or for events: cargo run --bin client [::1]:4433 /events");

  tako::serve_h3(router, "[::]:4433", Some("cert.pem"), Some("key.pem")).await;

  Ok(())
}
