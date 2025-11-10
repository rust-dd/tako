//! GraphiQL HTML responder and helper for Tako.
//!
//! Enable with the `graphiql` feature. This module provides a `graphiql()` function that
//! returns an HTML page rendering the GraphiQL UI, wired to your GraphQL and WS endpoints.
#![cfg(feature = "graphiql")]

use http::{HeaderValue, header};

use crate::{body::TakoBody, responder::Responder, types::Response};

/// Response wrapper for GraphiQL HTML.
pub struct GraphiQL(pub(crate) String);

impl Responder for GraphiQL {
  fn into_response(self) -> Response {
    let mut res = Response::new(TakoBody::from(self.0));
    res.headers_mut().insert(
      header::CONTENT_TYPE,
      HeaderValue::from_static("text/html; charset=utf-8"),
    );
    res
  }
}

/// Build a GraphiQL HTML response.
///
/// - `endpoint`: HTTP endpoint for GraphQL queries/mutations (e.g., "/graphql")
/// - `subscription_endpoint`: optional WS URL for subscriptions (e.g., "ws://localhost:8080/ws")
pub fn graphiql(endpoint: &str, subscription_endpoint: Option<&str>) -> GraphiQL {
  let mut builder = async_graphql::http::GraphiQLSource::build().endpoint(endpoint);
  if let Some(ws) = subscription_endpoint {
    builder = builder.subscription_endpoint(ws);
  }
  GraphiQL(builder.finish())
}
