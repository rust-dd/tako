//! WebSocket subprotocol negotiation for `GraphQL` subscriptions:
//! the [`GraphQLProtocol`] extractor and its rejection type.

use std::str::FromStr;

use async_graphql::http::WebSocketProtocols;
use http::StatusCode;
use http::header;

use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Extracted WebSocket protocol for `GraphQL` subscriptions.
pub struct GraphQLProtocol(pub WebSocketProtocols);

#[derive(Debug)]
pub struct GraphQLProtocolRejection;

impl Responder for GraphQLProtocolRejection {
  fn into_response(self) -> Response {
    (
      StatusCode::BAD_REQUEST,
      "Missing or invalid Sec-WebSocket-Protocol",
    )
      .into_response()
  }
}

impl<'a> FromRequestParts<'a> for GraphQLProtocol {
  type Error = GraphQLProtocolRejection;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      parts
        .headers
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(|protocols| {
          protocols
            .split(',')
            .find_map(|p| WebSocketProtocols::from_str(p.trim()).ok())
        })
        .map(GraphQLProtocol)
        .ok_or(GraphQLProtocolRejection),
    )
  }
}

impl<'a> FromRequest<'a> for GraphQLProtocol {
  type Error = GraphQLProtocolRejection;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(
      req
        .headers()
        .get(header::SEC_WEBSOCKET_PROTOCOL)
        .and_then(|v| v.to_str().ok())
        .and_then(|protocols| {
          protocols
            .split(',')
            .find_map(|p| WebSocketProtocols::from_str(p.trim()).ok())
        })
        .map(GraphQLProtocol)
        .ok_or(GraphQLProtocolRejection),
    )
  }
}
