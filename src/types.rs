use std::pin::Pin;

use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;
use hyper::body::Incoming;

use crate::body::TakoBody;

pub type BoxBody = UnsyncBoxBody<Bytes, BoxError>;
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type BoxedHandlerFuture = Pin<Box<dyn Future<Output = Response> + Send + 'static>>;

pub type Request = hyper::Request<Incoming>;
pub type Response = hyper::Response<TakoBody>;

pub trait AppState: Clone + Default + Send + Sync + 'static {}
