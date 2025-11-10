use anyhow::Result;
use async_graphql::futures_util::Stream;
use async_graphql::futures_util::stream;
use async_graphql::{Context, EmptyMutation, Object, Schema, Subscription};
use std::time::Duration;
use tako::extractors::FromRequest;
use tako::graphql::{GraphQLRequest, GraphQLResponse, GraphQLSubscription};
use tako::types::Request as TakoRequest;
use tako::{Method, router::Router};
use tokio::net::TcpListener;

struct QueryRoot;

#[Object]
impl QueryRoot {
  async fn hello(&self) -> &str {
    "Hello, GraphQL!"
  }
}

struct SubscriptionRoot;

#[Subscription]
impl SubscriptionRoot {
  async fn tick(&self, _ctx: &Context<'_>) -> impl Stream<Item = i32> {
    stream::unfold(0, |i| async move {
      tokio::time::sleep(Duration::from_millis(200)).await;
      Some((i, i + 1))
    })
  }
}

type AppSchema = Schema<QueryRoot, EmptyMutation, SubscriptionRoot>;

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let schema = Schema::build(QueryRoot, EmptyMutation, SubscriptionRoot).finish();

  let mut router = Router::new();

  // POST /graphql
  router.route(Method::POST, "/graphql", {
    let schema = schema.clone();
    move |mut req: TakoRequest| {
      let schema = schema.clone();
      async move {
        let gql_req: GraphQLRequest = GraphQLRequest::from_request(&mut req).await.unwrap();
        let resp = schema.execute(gql_req.0).await;
        GraphQLResponse(resp)
      }
    }
  });

  // GET /ws for subscriptions (graphql-transport-ws or graphql-ws)
  router.route(Method::GET, "/ws", {
    let schema = schema.clone();
    move |req: TakoRequest| {
      let schema = schema.clone();
      async move { GraphQLSubscription::new(req, schema) }
    }
  });

  println!("GraphQL: POST http://127.0.0.1:8080/graphql");
  println!("Subscriptions (WS): ws://127.0.0.1:8080/ws");

  tako::serve(listener, router).await;
  Ok(())
}
