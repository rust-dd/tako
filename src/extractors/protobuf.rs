#![cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
//! Protobuf request body extraction and deserialization for API endpoints.
//!
//! This module provides extractors for parsing Protocol Buffer request bodies into strongly-typed
//! Rust structures using prost. It validates Content-Type headers, reads request bodies efficiently,
//! and provides detailed error information for malformed protobuf data or incorrect content types.
//! The extractor integrates seamlessly with prost-generated message types for automatic protobuf
//! deserialization of complex data structures.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::protobuf::Protobuf;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use prost::Message;
//!
//! #[derive(Clone, PartialEq, Message)]
//! struct CreateUserRequest {
//!     #[prost(string, tag = "1")]
//!     pub name: String,
//!     #[prost(string, tag = "2")]
//!     pub email: String,
//!     #[prost(uint32, tag = "3")]
//!     pub age: u32,
//! }
//!
//! async fn create_user_handler(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let user_data: Protobuf<CreateUserRequest> = Protobuf::from_request(&mut req).await?;
//!
//!     // Access the deserialized data
//!     println!("Creating user: {} ({})", user_data.0.name, user_data.0.email);
//!
//!     Ok(format!("User {} created successfully", user_data.0.name))
//! }
//! ```

use http::StatusCode;
use http_body_util::BodyExt;
use prost::Message;

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// Protobuf request body extractor with automatic deserialization.
pub struct Protobuf<T>(pub T);

/// Error types for Protobuf extraction and deserialization.
#[derive(Debug)]
pub enum ProtobufError {
  /// Content-Type header is not application/x-protobuf or application/protobuf.
  InvalidContentType,
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Failed to read the request body (network error, timeout, etc.).
  BodyReadError(String),
  /// Protobuf deserialization failed (invalid format, unknown fields, etc.).
  ProtobufDecodeError(String),
}

impl Responder for ProtobufError {
  /// Converts Protobuf extraction errors into appropriate HTTP error responses.
  fn into_response(self) -> crate::types::Response {
    match self {
      ProtobufError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected application/x-protobuf or application/protobuf",
      )
        .into_response(),
      ProtobufError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing content type header").into_response()
      }
      ProtobufError::BodyReadError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to read request body: {}", err),
      )
        .into_response(),
      ProtobufError::ProtobufDecodeError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to decode protobuf: {}", err),
      )
        .into_response(),
    }
  }
}

impl<T> Responder for Protobuf<T>
where
  T: Message,
{
  /// Converts the wrapped protobuf message into an HTTP response.
  fn into_response(self) -> crate::types::Response {
    let buf = self.0.encode_to_vec();
    let mut res = crate::types::Response::new(crate::body::TakoBody::from(buf));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("application/x-protobuf"),
    );
    res
  }
}

/// Checks if the Content-Type header indicates protobuf content.
fn is_protobuf_content_type(headers: &http::HeaderMap) -> bool {
  headers
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .map(|ct| {
      ct == "application/x-protobuf"
        || ct == "application/protobuf"
        || ct.starts_with("application/x-protobuf;")
        || ct.starts_with("application/protobuf;")
    })
    .unwrap_or(false)
}

impl<'a, T> FromRequest<'a> for Protobuf<T>
where
  T: Message + Default + Send + 'static,
{
  type Error = ProtobufError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if !is_protobuf_content_type(req.headers()) {
        return Err(ProtobufError::InvalidContentType);
      }

      let body_bytes = req
        .body_mut()
        .collect()
        .await
        .map_err(|e| ProtobufError::BodyReadError(e.to_string()))?
        .to_bytes();

      let data = T::decode(&body_bytes[..])
        .map_err(|e| ProtobufError::ProtobufDecodeError(e.to_string()))?;

      Ok(Protobuf(data))
    }
  }
}
