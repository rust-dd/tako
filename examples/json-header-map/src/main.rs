use anyhow::Result;
use serde::Deserialize;
use serde::Serialize;
use tako::Method;
use tako::extractors::header_map::HeaderMap;
use tako::extractors::json::Json;
use tako::router::Router;
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
/// Demonstrates using both `Json` and `HeaderMap` extractors in the handler signature.
async fn echo_with_headers(
  HeaderMap(headers): HeaderMap,
  Json(payload): Json<Input>,
) -> Json<Output> {
  let user_agent = headers
    .get("user-agent")
    .and_then(|v| v.to_str().ok())
    .map(|s| s.to_string());

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
