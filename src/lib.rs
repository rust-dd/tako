use tokio::net::TcpListener;

use crate::router::Router;

pub mod body;
pub mod handler;
pub mod responder;
pub mod router;
pub mod server;
pub mod types;

pub async fn serve(listener: TcpListener, router: Router) {
    server::run(listener, router).await.unwrap();
}
