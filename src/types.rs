use bytes::Bytes;
use http_body_util::combinators::UnsyncBoxBody;

pub type BoxBody = UnsyncBoxBody<Bytes, BoxError>;
pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub trait AppState: Clone + Default + Send + Sync + 'static {}
