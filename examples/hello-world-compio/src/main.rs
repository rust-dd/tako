use anyhow::Result;
use compio::net::TcpListener;
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;

async fn hello_world() -> impl Responder {
  "Hello, World!".into_response()
}

#[compio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", hello_world);

  tako::serve(listener, router).await;

  Ok(())
}
