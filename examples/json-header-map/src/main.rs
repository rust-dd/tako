use anyhow::Result;
use serde::{Deserialize, Serialize};
use tako::extractors::{header_map::HeaderMap, json::Json, FromRequest};
use tako::{router::Router, types::Request, Method};
use tokio::net::TcpListener;

#[derive(Deserialize)]
struct Input {
  name: String,
}

#[derive(Serialize)]
struct Output {
  name: String,
  user_agent: Option<String>,
}

/// POST /echo
/// Body: {"name": "Alice"}
///
/// Demonstrates using both `Json` and `HeaderMap<'_>` extractors inside a handler.
async fn echo_with_headers(mut req: Request) -> Json<Output> {
  // First, grab the headers via the lifetime-based extractor
  let HeaderMap(headers): HeaderMap<'_> = HeaderMap::from_request(&mut req)
    .await
    .expect("failed to extract headers");

  let user_agent = headers
    .get("user-agent")
    .and_then(|v| v.to_str().ok())
    .map(|s| s.to_string());

  // Then, read the JSON body
  let Json(payload): Json<Input> = Json::from_request(&mut req)
    .await
    .expect("invalid JSON body");

  Json(Output {
    name: payload.name,
    user_agent,
  })
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::POST, "/echo", echo_with_headers);

  tako::serve(listener, router).await;

  Ok(())
}
