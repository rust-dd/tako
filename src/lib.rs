use bytes::Bytes;
use http_body_util::Empty;
use hyper::body::Body;
use tokio::net::TcpListener;

use crate::router::Router;

pub mod handler;
pub mod router;
pub mod server;

pub async fn serve<B>(listener: TcpListener, router: Router<B>)
where
    B: Body + From<Empty<Bytes>> + Send + 'static,
    B::Data: Send,
    B::Error: Into<Box<dyn std::error::Error + Send + Sync>>,
{
    server::run(listener, router).await.unwrap();
}
