use anyhow::Result;
use tako::{Method, plugins::cors::CorsPlugin, responder::Responder, router::Router};
use tokio::net::TcpListener;

async fn hello_world() -> impl Responder {
  "Hello, World!".into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.plugin(CorsPlugin::default());
  router.route(Method::GET, "/", hello_world);

  tako::serve(listener, router).await;

  Ok(())
}
