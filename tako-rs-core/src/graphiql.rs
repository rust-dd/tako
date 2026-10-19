//! `GraphiQL` IDE page for a `GraphQL` endpoint.

use http::HeaderValue;
use http::header;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Response;

/// The `GraphiQL` HTML page built by [`graphiql`].
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

/// Builds the IDE page for `endpoint`; subscriptions go to the WebSocket `subscription_endpoint`.
pub fn graphiql(endpoint: &str, subscription_endpoint: Option<&str>) -> GraphiQL {
  let mut builder = async_graphql::http::GraphiQLSource::build().endpoint(endpoint);
  if let Some(ws) = subscription_endpoint {
    builder = builder.subscription_endpoint(ws);
  }
  GraphiQL(builder.finish())
}
