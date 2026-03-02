use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt::init();

  println!("Starting UDP echo server on 127.0.0.1:9000");
  println!("Send datagrams with: echo -n 'hello' | nc -u 127.0.0.1 9000");

  tako::server_udp::serve_udp("127.0.0.1:9000", |data, addr, socket| {
    Box::pin(async move {
      let msg = String::from_utf8_lossy(&data);
      println!("[{addr}] Received ({} bytes): {msg}", data.len());

      // Echo back
      let _ = socket.send_to(&data, addr).await;
    })
  })
  .await?;

  Ok(())
}
