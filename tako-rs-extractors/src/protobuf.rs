//! Protocol Buffers bodies decoded with `prost`.
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
//! async fn create_user_handler(mut req: Request) -> Result<String, tako::extractors::protobuf::ProtobufError> {
//!     let user_data: Protobuf<CreateUserRequest> = Protobuf::from_request(&mut req).await?;
//!
//!     // Access the deserialized data
//!     println!("Creating user: {} ({})", user_data.0.name, user_data.0.email);
//!
//!     Ok(format!("User {} created successfully", user_data.0.name))
//! }
//! ```

use http::StatusCode;
use prost::Message;
use tako_rs_core::extractors::Entries;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::body::collect_body;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// A message decoded from an `application/x-protobuf` or `application/protobuf` body; the
/// route's body limit applies.
#[doc(alias = "protobuf")]
pub struct Protobuf<T>(pub T);

/// Rejection of [`Protobuf`]; a body read error keeps its own status, everything else is 400.
#[derive(Debug)]
pub enum ProtobufError {
  /// Also returned when the `Content-Type` header is missing.
  InvalidContentType,
  BodyReadError(tako_rs_core::extractors::body::BodyReadError),
  ProtobufDecodeError(String),
}

impl Responder for ProtobufError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      ProtobufError::InvalidContentType => (
        StatusCode::BAD_REQUEST,
        "Invalid content type; expected application/x-protobuf or application/protobuf",
      )
        .into_response(),
      ProtobufError::BodyReadError(err) => {
        (err.status(), format!("Failed to read request body: {err}")).into_response()
      }
      ProtobufError::ProtobufDecodeError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Failed to decode protobuf: {err}"),
      )
        .into_response(),
    }
  }
}

impl<T> Responder for Protobuf<T>
where
  T: Message,
{
  /// Encodes the message with `Content-Type: application/x-protobuf`.
  fn into_response(self) -> tako_rs_core::types::Response {
    let buf = self.0.encode_to_vec();
    let mut res = tako_rs_core::types::Response::new(tako_rs_core::body::TakoBody::from(buf));
    res.headers_mut().insert(
      http::header::CONTENT_TYPE,
      http::HeaderValue::from_static("application/x-protobuf"),
    );
    res
  }
}

fn is_protobuf_content_type(headers: &http::HeaderMap) -> bool {
  headers
    .get(http::header::CONTENT_TYPE)
    .and_then(|v| v.to_str().ok())
    .is_some_and(|ct| {
      ct == "application/x-protobuf"
        || ct == "application/protobuf"
        || ct.starts_with("application/x-protobuf;")
        || ct.starts_with("application/protobuf;")
    })
}

impl<'a, T> FromRequest<'a> for Protobuf<T>
where
  T: Message + Default + Send + 'static,
{
  type Error = ProtobufError;
  const ENTRIES: Entries = Entries::BODY_LIMIT;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      if !is_protobuf_content_type(req.headers()) {
        return Err(ProtobufError::InvalidContentType);
      }

      let body_bytes = collect_body(req)
        .await
        .map_err(ProtobufError::BodyReadError)?;

      let data = T::decode(&body_bytes[..])
        .map_err(|e| ProtobufError::ProtobufDecodeError(e.to_string()))?;

      Ok(Protobuf(data))
    }
  }
}
