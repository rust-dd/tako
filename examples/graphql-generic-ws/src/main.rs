use anyhow::Result;
use async_graphql::futures_util::{Stream, stream};
use async_graphql::{Context, EmptyMutation, Object, Schema, Subscription};
use http::{HeaderValue, StatusCode, header};
use hyper_util::rt::TokioIo;
use std::time::Duration;
use tako::extractors::FromRequest;
use tako::graphql::{GraphQLRequest, GraphQLResponse, GraphQLWebSocket};
use tako::responder::Responder;
use tako::types::{Request as TakoRequest, Response as TakoResponse};
use tako::{Method, router::Router};
use tokio::net::TcpListener;
use tokio_tungstenite::{WebSocketStream, tungstenite::protocol::Role};

struct QueryRoot;

#[Object]
impl QueryRoot {
  async fn hello(&self) -> &str {
    "Hello, GraphQL Generic WS!"
  }
}

struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
  async fn tick(&self, _ctx: &Context<'_>) -> impl Stream<Item = i32> {
    stream::unfold(0, |i| async move {
      tokio::time::sleep(Duration::from_millis(250)).await;
      Some((i, i + 1))
    })
  }
}

type AppSchema = Schema<QueryRoot, EmptyMutation, SubscriptionRoot>;

fn get_schema() -> AppSchema {
  tako::state::get_state::<AppSchema>()
    .expect("schema missing")
    .as_ref()
    .clone()
}

async fn graphql_http(GraphQLRequest(gql): GraphQLRequest) -> GraphQLResponse {
  let schema = get_schema();
  GraphQLResponse(schema.execute(gql).await)
}

async fn ws_generic(mut req: TakoRequest) -> TakoResponse {
  // Extract protocol using the extractor internally
  let tako::graphql::GraphQLProtocol(protocol) =
    match tako::graphql::GraphQLProtocol::from_request(&mut req).await {
      Ok(p) => p,
      Err(_) => {
        return (
          StatusCode::BAD_REQUEST,
          "Missing or invalid Sec-WebSocket-Protocol",
        )
          .into_response();
      }
    };
  // Compute Sec-WebSocket-Accept (RFC 6455)
  let key = match req.headers().get("Sec-WebSocket-Key") {
    Some(k) => k,
    None => return (StatusCode::BAD_REQUEST, "Missing Sec-WebSocket-Key").into_response(),
  };
  let accept = {
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use sha1::{Digest, Sha1};
    let mut sha1 = Sha1::new();
    sha1.update(key.as_bytes());
    sha1.update(b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
    STANDARD.encode(sha1.finalize())
  };

  // Build upgrade response echoing the chosen subprotocol
  let response = http::Response::builder()
    .status(StatusCode::SWITCHING_PROTOCOLS)
    .header(header::UPGRADE, "websocket")
    .header(header::CONNECTION, "Upgrade")
    .header("Sec-WebSocket-Accept", accept)
    .header(
      header::SEC_WEBSOCKET_PROTOCOL,
      HeaderValue::from_static(protocol.sec_websocket_protocol()),
    )
    .body(tako::body::TakoBody::empty())
    .unwrap();

  // Spawn the WS task using the generic GraphQLWebSocket driver
  if let Some(on_upgrade) = req.extensions().get::<hyper::upgrade::OnUpgrade>().cloned() {
    tokio::spawn(async move {
      if let Ok(upgraded) = on_upgrade.await {
        let upgraded = TokioIo::new(upgraded);
        let ws: WebSocketStream<TokioIo<hyper::upgrade::Upgraded>> =
          WebSocketStream::from_raw_socket(upgraded, Role::Server, None).await;

        let driver = GraphQLWebSocket::new(ws, get_schema(), protocol);
        driver.serve().await;
      }
    });
  }

  response
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8081").await?;

  let schema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot).finish();

  let mut router = Router::new();
  router.state(schema.clone());
  router.route(Method::POST, "/graphql", graphql_http);
  router.route(Method::GET, "/ws-generic", ws_generic);

  println!("GraphQL (HTTP): POST http://127.0.0.1:8081/graphql");
  println!("Generic WS: ws://127.0.0.1:8081/ws-generic");

  tako::serve(listener, router).await;
  Ok(())
}
