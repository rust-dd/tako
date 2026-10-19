use std::time::Duration;

use anyhow::Result;
use async_graphql::Context;
use async_graphql::EmptyMutation;
use async_graphql::Object;
use async_graphql::Schema;
use async_graphql::Subscription;
use async_graphql::futures_util::Stream;
use async_graphql::futures_util::stream;
use http::StatusCode;
use tako::Method;
use tako::extractors::FromRequest;
use tako::extractors::state::State;
use tako::graphql::GraphQLProtocol;
use tako::graphql::GraphQLRequest;
use tako::graphql::GraphQLResponse;
use tako::graphql::GraphQLWebSocket;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request as TakoRequest;
use tako::types::Response as TakoResponse;
use tako::ws::WebSocketUpgrade;
use tokio::net::TcpListener;

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

async fn graphql_http(
  State(schema): State<AppSchema>,
  GraphQLRequest(gql): GraphQLRequest,
) -> GraphQLResponse {
  GraphQLResponse(schema.execute(gql).await)
}

async fn ws_generic(State(schema): State<AppSchema>, mut req: TakoRequest) -> TakoResponse {
  let Ok(GraphQLProtocol(protocol)) = GraphQLProtocol::from_request(&mut req).await else {
    return (
      StatusCode::BAD_REQUEST,
      "Missing or invalid Sec-WebSocket-Protocol",
    )
      .into_response();
  };
  let schema = schema.as_ref().clone();
  WebSocketUpgrade::new(req, move |ws| {
    GraphQLWebSocket::new(ws, schema, protocol).serve()
  })
  .protocols([protocol.sec_websocket_protocol()])
  .into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8081").await?;

  let schema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot).finish();

  let mut router = Router::new();
  router.with_state(schema.clone());
  router.route(Method::POST, "/graphql", graphql_http);
  router.route(Method::GET, "/ws-generic", ws_generic);

  println!("GraphQL (HTTP): POST http://127.0.0.1:8081/graphql");
  println!("Generic WS: ws://127.0.0.1:8081/ws-generic");

  tako::Server::builder()
    .build()
    .spawn_http(listener, router)
    .result()
    .await
    .expect("HTTP server failed");
  Ok(())
}
