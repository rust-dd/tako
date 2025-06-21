use std::pin::Pin;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;

use crate::body::TakoBody;

/// Represents an HTTP request with an incoming body.
pub type Request = hyper::Request<Incoming>;

/// Represents an HTTP response with a `TakoBody`.
pub type Response = hyper::Response<TakoBody>;

/// A boxed body type used for HTTP responses, combining `Bytes` data with a boxed error type.
pub type BoxedBody = UnsyncBoxBody<Bytes, BoxedError>;

/// A boxed error type that can be sent across threads and is compatible with dynamic dispatch.
pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;

/// A boxed future that resolves to an HTTP response. The default type is `Response`.
pub type BoxedResponseFuture<R = Response> = Pin<Box<dyn Future<Output = R> + Send + 'static>>;

/// A boxed future that resolves to either a `Request` or a `Response` in case of an error.
pub type BoxedRequestFuture =
    Pin<Box<dyn Future<Output = Result<Request, Response>> + Send + 'static>>;

/// A boxed middleware function that processes a `Request` and returns either a modified `Request`
/// or a `Response` in case of an error.
pub type BoxedMiddleware = Box<
    dyn Fn(Request) -> Pin<Box<dyn Future<Output = Result<Request, Response>> + Send>>
        + Send
        + Sync
        + 'static,
>;
