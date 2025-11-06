use anyhow::Result;
use serde::Serialize;
use tako::{Method, extractors::json::Json, router::Router};
use tokio::net::TcpListener;

#[derive(Serialize)]
pub struct HealthCheck {
    status: String,
}

async fn health() -> Json<HealthCheck> {
    Json(HealthCheck {
        status: "OK".to_string(),
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::GET, "/health", health);

    println!("Server running at http://127.0.0.1:8080");
    tako::serve(listener, router).await;

    Ok(())
}
