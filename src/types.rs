use std::pin::Pin;

use bytes::Bytes;
use http::Response;
use http_body_util::combinators::UnsyncBoxBody;

use crate::body::TakoBody;

pub type BoxBody = UnsyncBoxBody<Bytes, BoxError>;
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;
pub type Fut<'a> = Pin<Box<dyn Future<Output = Response<TakoBody>> + Send + 'a>>;

pub trait AppState: Clone + Default + Send + Sync + 'static {}
