use std::sync::Arc;

use bytes::Bytes;
use futures_util::future::BoxFuture;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;

use crate::{body::TakoBody, middleware::Next};

/// Represents an HTTP request with an incoming body.
pub type Request = hyper::Request<Incoming>;

/// Represents an HTTP response with a `TakoBody`.
pub type Response = hyper::Response<TakoBody>;

/// A boxed body type used for HTTP responses, combining `Bytes` data with a boxed error type.
pub(crate) type BoxBody = UnsyncBoxBody<Bytes, BoxError>;

/// A boxed error type that can be sent across threads and is compatible with dynamic dispatch.
pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// A boxed middleware type that can be sent across threads and is compatible with dynamic dispatch.
pub type BoxMiddleware = Arc<dyn Fn(Request, Next) -> BoxFuture<'static, Response> + Send + Sync>;
