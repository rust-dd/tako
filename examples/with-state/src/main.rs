use anyhow::Result;
use tako::Method;
use tako::extractors::state::State;
use tako::responder::Responder;
use tako::router::Router;
use tokio::net::TcpListener;

async fn hello_world(
  State(names): State<Vec<&'static str>>,
  State(age): State<u32>,
  State(city): State<&'static str>,
) -> impl Responder {
  format!(
    "Hello , World! Names: {:?}, Age: {}, City: {}",
    names, age, city
  )
  .into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.with_state(vec!["Alice", "Bob", "Charlie"]);
  router.with_state(25_u32);
  router.with_state("New York");
  router.route(Method::GET, "/", hello_world);

  tako::Server::builder()
    .build()
    .spawn_http(listener, router)
    .result()
    .await
    .expect("HTTP server failed");

  Ok(())
}
