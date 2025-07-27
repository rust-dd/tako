use anyhow::Result;
use tako::{Method, responder::Responder, router::Router, types::Request};
use tokio::net::TcpListener;

async fn hello_world(_: Request) -> impl Responder {
    "Hello, World!".into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = Router::new();
    router.route(Method::GET, "/", hello_world);

    println!("Server running at http://127.0.0.1:8080");
    tako::serve(listener, router).await;

    Ok(())
}
