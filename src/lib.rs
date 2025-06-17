use tokio::net::TcpListener;

use crate::{router::Router, types::AppState};

pub mod body;
pub mod extractors;
pub mod handler;
pub mod middleware;
pub mod responder;
pub mod route;
pub mod router;
pub mod server;
pub mod types;

pub async fn serve<S>(listener: TcpListener, router: Router<'static, S>)
where
    S: AppState,
{
    server::run(listener, router).await.unwrap();
}
