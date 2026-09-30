#![cfg_attr(docsrs, doc(cfg(feature = "grpc")))]

//! gRPC over HTTP/2: unary, server-streaming, client-streaming, and
//! bidirectional RPCs.
//!
//! Each method is an ordinary `POST /<package>.<Service>/<Method>` route.
//! [`GrpcRequest`](crate::grpc::GrpcRequest) and
//! [`GrpcResponse`](crate::grpc::GrpcResponse) handle one message each way;
//! [`GrpcServerStream`](crate::grpc::GrpcServerStream) streams replies,
//! [`GrpcClientStream`](crate::grpc::GrpcClientStream) reads a request stream,
//! and [`GrpcBidi`](crate::grpc::GrpcBidi) does both at once.
//! `Option<GrpcDeadline>` extracts the client's `grpc-timeout`.
//!
//! # Examples
//!
//! ```rust
//! use futures_util::StreamExt;
//! use prost::Message;
//! use tako::Method;
//! use tako::grpc::{GrpcBidi, GrpcClientStream, GrpcDeadline, GrpcRequest};
//! use tako::grpc::{GrpcResponse, GrpcServerStream, GrpcStatus};
//! use tako::responder::Responder;
//! use tako::router::Router;
//!
//! #[derive(Clone, PartialEq, Message)]
//! pub struct Number {
//!   #[prost(int64, tag = "1")]
//!   pub value: i64,
//! }
//!
//! async fn double(req: GrpcRequest<Number>) -> GrpcResponse<Number> {
//!   GrpcResponse::ok(Number { value: req.message.value * 2 })
//! }
//!
//! async fn count(deadline: Option<GrpcDeadline>, req: GrpcRequest<Number>) -> impl Responder {
//!   let numbers = futures_util::stream::iter(1..=req.message.value).map(|value| Ok(Number { value }));
//!   GrpcServerStream::new(numbers).with_deadline(deadline)
//! }
//!
//! async fn sum(mut numbers: GrpcClientStream<Number>) -> Result<GrpcResponse<Number>, GrpcStatus> {
//!   let mut value = 0;
//!   while let Some(number) = numbers.next().await {
//!     value += number?.value;
//!   }
//!   Ok(GrpcResponse::ok(Number { value }))
//! }
//!
//! async fn echo(bidi: GrpcBidi<Number, Number>) -> impl Responder {
//!   bidi.respond(|numbers| {
//!     numbers.map(|number| number.map(|n| Number { value: n.value * 2 }).map_err(GrpcStatus::from))
//!   })
//! }
//!
//! let mut router = Router::new();
//! router.route(Method::POST, "/math.Math/Double", double);
//! router.route(Method::POST, "/math.Math/Count", count);
//! router.route(Method::POST, "/math.Math/Sum", sum);
//! router.route(Method::POST, "/math.Math/Echo", echo);
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
#[cfg(test)]
mod tests;
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
