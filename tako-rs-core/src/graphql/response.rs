use async_graphql::BatchResponse as GqlBatchResponse;
use http::HeaderValue;
use http::StatusCode;
use http::header;

use crate::body::TakoBody;
use crate::responder::Responder;
use crate::types::Response;

/// Sends an `async_graphql::Response` as an `application/json` body.
pub struct GraphQLResponse(pub async_graphql::Response);

impl From<async_graphql::Response> for GraphQLResponse {
  fn from(value: async_graphql::Response) -> Self {
    Self(value)
  }
}

impl Responder for GraphQLResponse {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
  }
}

/// Sends an `async_graphql::BatchResponse` as an `application/json` body.
pub struct GraphQLBatchResponse(pub GqlBatchResponse);

impl From<GqlBatchResponse> for GraphQLBatchResponse {
  fn from(value: GqlBatchResponse) -> Self {
    Self(value)
  }
}

impl Responder for GraphQLBatchResponse {
  fn into_response(self) -> Response {
    match serde_json::to_vec(&self.0) {
      Ok(buf) => {
        let mut res = Response::new(TakoBody::from(buf));
        res.headers_mut().insert(
          header::CONTENT_TYPE,
          HeaderValue::from_static(mime::APPLICATION_JSON.as_ref()),
        );
        res
      }
      Err(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()).into_response(),
    }
  }
}
