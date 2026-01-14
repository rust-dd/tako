use anyhow::Result;
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;

async fn hello_world() -> impl Responder {
  "Hello, HTTP/3 World!".into_response()
}

async fn json_example() -> impl Responder {
  r#"{"message": "Hello from HTTP/3!", "protocol": "h3"}"#.into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  // HTTP/3 requires TLS certificates
  // You can generate self-signed certificates for testing with:
  // openssl req -x509 -newkey rsa:4096 -keyout key.pem -out cert.pem -days 365 -nodes -subj "/CN=localhost"

  let mut router = Router::new();
  router.route(Method::GET, "/", hello_world);
  router.route(Method::GET, "/json", json_example);

  println!("Starting HTTP/3 server on [::]:4433");
  println!("Test with: curl --http3 -k https://localhost:4433/");

  tako::serve_h3(router, "[::]:4433", Some("cert.pem"), Some("key.pem")).await;

  Ok(())
}
