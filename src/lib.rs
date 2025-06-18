use tokio::net::TcpListener;

use crate::router::Router;

pub mod body;
pub mod extractors;
pub mod handler;
pub mod middleware;
pub mod responder;
pub mod route;
pub mod router;
pub mod server;
pub mod state;
pub mod types;

pub async fn serve(listener: TcpListener, router: Router<'static>) {
    server::run(listener, router).await.unwrap();
}
