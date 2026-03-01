//! TLS example using compio runtime.
//!
//! Run with: cargo run --example tls-compio --features compio-tls
//!
//! Requires cert.pem and key.pem in the working directory.
//! Generate self-signed certs for testing:
//!   openssl req -x509 -newkey rsa:2048 -keyout key.pem -out cert.pem -days 365 -nodes -subj '/CN=localhost'

use anyhow::Result;
use compio::net::TcpListener;
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

async fn hello(_: Request) -> impl Responder {
  "Hello, Secure Compio World!"
}

async fn health(_: Request) -> impl Responder {
  (http::StatusCode::OK, "OK")
}

#[compio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8443").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", hello);
  router.route(Method::GET, "/health", health);

  println!("TLS (compio) server running at https://127.0.0.1:8443");

  tako::serve_tls(listener, router, Some("cert.pem"), Some("key.pem")).await;

  Ok(())
}
