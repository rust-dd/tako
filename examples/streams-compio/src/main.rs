//! Streaming / SSE example using compio runtime.
//!
//! Run with: cargo run --example streams-compio --features compio

use std::convert::Infallible;

use anyhow::Result;
use bytes::Bytes;
use futures_util::StreamExt;
use futures_util::stream;
use http::StatusCode;
use http::header;
use http_body::Frame;
use tako::Method;
use tako::body::TakoBody;
use tako::responder::Responder;
use tako::router::Router;
use tako::sse::Sse;
use tako::types::Request;

/// Streams numbers 0-9 as raw text
async fn numbers(_: Request) -> impl Responder {
  let s = stream::iter(0u8..=9)
    .map(|n| Ok::<_, Infallible>(Bytes::from(format!("{}\n", n))));

  http::Response::builder()
    .status(StatusCode::OK)
    .header(header::CONTENT_TYPE, "text/event-stream")
    .header(header::CACHE_CONTROL, "no-cache")
    .header(header::CONNECTION, "keep-alive")
    .body(TakoBody::from_stream(s))
    .unwrap()
    .into_response()
}

/// Streams JSON ticks, with an intentional error every 5th tick
async fn json_ticks(_: Request) -> impl Responder {
  let s = stream::iter(0u64..20).map(|i| {
    if i % 5 == 4 {
      Err::<Frame<Bytes>, _>(std::io::Error::new(std::io::ErrorKind::Other, "boom"))
    } else {
      let payload = format!("{{\"tick\":{}}}", i);
      Ok(Frame::data(Bytes::from(payload)))
    }
  });

  http::Response::builder()
    .status(StatusCode::OK)
    .header(header::CONTENT_TYPE, "text/event-stream")
    .header(header::CACHE_CONTROL, "no-cache")
    .header(header::CONNECTION, "keep-alive")
    .body(TakoBody::from_try_stream(s))
    .unwrap()
    .into_response()
}

/// SSE-formatted ticker
async fn ticker(_: Request) -> impl Responder {
  let s = stream::iter(0u64..10).map(|i| {
    let msg = format!("tick: {i}");
    Bytes::from(msg)
  });

  Sse::new(s)
}

#[compio::main]
async fn main() -> Result<()> {
  let listener = compio::net::TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", numbers);
  router.route(Method::GET, "/json", json_ticks);
  router.route(Method::GET, "/events", ticker);

  println!("Streams (compio) server running at:");
  println!("  - http://127.0.0.1:8080/       (numbers)");
  println!("  - http://127.0.0.1:8080/json    (json ticks)");
  println!("  - http://127.0.0.1:8080/events  (SSE ticker)");

  tako::serve(listener, router).await;
  Ok(())
}
