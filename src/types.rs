use std::pin::Pin;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;

use crate::body::TakoBody;

pub type Request = hyper::Request<Incoming>;
pub type Response = hyper::Response<TakoBody>;

pub type BoxedBody = UnsyncBoxBody<Bytes, BoxedError>;
pub type BoxedError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedResponseFuture<R = Response> = Pin<Box<dyn Future<Output = R> + Send + 'static>>;
pub type BoxedRequestFuture =
    Pin<Box<dyn Future<Output = Result<Request, Response>> + Send + 'static>>;
pub type BoxedMiddleware = Box<
    dyn Fn(Request) -> Pin<Box<dyn Future<Output = Result<Request, Response>> + Send>>
        + Send
        + Sync
        + 'static,
>;
