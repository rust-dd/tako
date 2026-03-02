use anyhow::Result;
use tako::proxy_protocol::{ProxyHeader, serve_http_with_proxy_protocol};
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;
use tako::Method;

async fn handler(req: Request) -> impl Responder {
  // The real client address is available as SocketAddr (from PROXY header)
  let real_addr = req
    .extensions()
    .get::<std::net::SocketAddr>()
    .map(|a| a.to_string())
    .unwrap_or_else(|| "unknown".into());

  // The full PROXY header is also available
  let proxy_info = req
    .extensions()
    .get::<ProxyHeader>()
    .map(|h| {
      format!(
        "version={:?}, transport={:?}, src={:?}, dst={:?}",
        h.version, h.transport, h.source, h.destination
      )
    })
    .unwrap_or_else(|| "no PROXY header".into());

  format!("Real client: {real_addr}\nPROXY header: {proxy_info}\n")
}

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt::init();

  let addr = "127.0.0.1:8080";
  let listener = tokio::net::TcpListener::bind(addr).await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", handler);

  println!("PROXY protocol HTTP server on {addr}");
  println!("This server expects a PROXY protocol header on each TCP connection.");
  println!("Test with HAProxy or a PROXY protocol client.");
  println!();
  println!("Quick test (v1 text format):");
  println!(r#"  echo -e "PROXY TCP4 192.168.1.100 10.0.0.1 56324 8080\r\nGET / HTTP/1.1\r\nHost: localhost\r\n\r\n" | nc 127.0.0.1 8080"#);

  serve_http_with_proxy_protocol(listener, router).await;

  Ok(())
}
