#![cfg_attr(docsrs, doc(cfg(feature = "grpc")))]

//! gRPC support for unary RPCs over HTTP/2.
//!
//! Provides `GrpcRequest<T>` extractor and `GrpcResponse<T>` responder that
//! handle gRPC framing (length-prefixed protobuf messages) and integrate with
//! Tako's handler system.
//!
//! # Examples
//!
//! ```rust,ignore
//! use tako::grpc::{GrpcRequest, GrpcResponse};
//! use prost::Message;
//!
//! #[derive(Clone, PartialEq, Message)]
//! struct HelloRequest {
//!     #[prost(string, tag = "1")]
//!     pub name: String,
//! }
//!
//! #[derive(Clone, PartialEq, Message)]
//! struct HelloReply {
//!     #[prost(string, tag = "1")]
//!     pub message: String,
//! }
//!
//! async fn say_hello(req: GrpcRequest<HelloRequest>) -> GrpcResponse<HelloReply> {
//!     GrpcResponse::ok(HelloReply {
//!         message: format!("Hello, {}!", req.message.name),
//!     })
//! }
//!
//! // Register on router:
//! // router.route(Method::POST, "/helloworld.Greeter/SayHello", say_hello);
//! ```

/// `grpc.health.v1` scaffolding.
pub mod health;
/// gRPC-specific interceptor pattern.
pub mod interceptor;
/// `grpc.reflection.v1` scaffolding.
pub mod reflection;
/// gRPC-Web bridge translating browser-friendly framing to canonical gRPC.
pub mod web;

mod framing;
mod message;
mod status;
mod streaming;
mod timeout;

pub use framing::GrpcError;
pub use framing::MAX_GRPC_MESSAGE_SIZE;
pub use framing::grpc_decode;
pub use framing::grpc_encode;
pub use message::GrpcRequest;
pub use message::GrpcResponse;
pub use status::GrpcStatus;
pub use status::GrpcStatusCode;
pub(crate) use status::build_grpc_error_response;
pub use streaming::GrpcBidi;
pub use streaming::GrpcClientStream;
pub use streaming::GrpcServerStream;
pub use timeout::GrpcDeadline;
pub use timeout::parse_grpc_timeout;
pub use timeout::read_grpc_deadline;
