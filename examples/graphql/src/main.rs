use std::time::Duration;

use anyhow::Result;
use async_graphql::Context;
use async_graphql::EmptyMutation;
use async_graphql::Object;
use async_graphql::Schema;
use async_graphql::Subscription;
use async_graphql::futures_util::Stream;
use async_graphql::futures_util::stream;
use tako::Method;
use tako::graphql::GraphQLRequest;
use tako::graphql::GraphQLResponse;
use tako::graphql::GraphQLSubscription;
use tako::graphql::graphiql;
use tako::router::Router;
use tako::types::Request as TakoRequest;
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

  // GraphiQL UI
  router.route(Method::GET, "/graphiql", move || async move {
    graphiql("/graphql", Some("ws://127.0.0.1:8080/ws"))
  });

  // POST /graphql
  router.route(Method::POST, "/graphql", {
    let schema = schema.clone();
    move |GraphQLRequest(req): GraphQLRequest| {
      let schema = schema.clone();
      async move {
        let resp = schema.execute(req).await;
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
  #[cfg(feature = "graphiql")]
  println!("GraphiQL UI: http://127.0.0.1:8080/graphiql");

  tako::serve(listener, router).await;
  Ok(())
}
