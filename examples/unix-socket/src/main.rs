use anyhow::Result;
use tako::middleware::IntoMiddleware;
use tako::middleware::request_id::RequestId;
use tako::responder::Responder;
use tako::router::Router;
use tako::server_unix::{UnixPeerAddr, serve_unix_http};
use tako::types::Request;
use tako::Method;

async fn hello(req: Request) -> impl Responder {
  let peer = req
    .extensions()
    .get::<UnixPeerAddr>()
    .map(|p| format!("{:?}", p.path))
    .unwrap_or_else(|| "unknown".into());

  format!("Hello from Unix socket! Peer: {peer}")
}

async fn health(_req: Request) -> impl Responder {
  (http::StatusCode::OK, "ok")
}

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt::init();

  let socket_path = "/tmp/tako-example.sock";

  let request_id = RequestId::new().into_middleware();

  let mut router = Router::new();
  router
    .route(Method::GET, "/", hello)
    .middleware(request_id.clone());
  router
    .route(Method::GET, "/health", health)
    .middleware(request_id);

  println!("Starting HTTP server on Unix socket: {socket_path}");
  println!("Test with: curl --unix-socket {socket_path} http://localhost/");
  println!("           curl --unix-socket {socket_path} http://localhost/health");

  serve_unix_http(socket_path, router).await;

  Ok(())
}
