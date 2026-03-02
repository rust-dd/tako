use anyhow::Result;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> Result<()> {
  tracing_subscriber::fmt::init();

  println!("Starting TCP echo server on 127.0.0.1:9001");
  println!("Connect with: nc 127.0.0.1 9001");

  tako::server_tcp::serve_tcp("127.0.0.1:9001", |mut stream, addr| {
    Box::pin(async move {
      println!("New connection from {addr}");
      let mut buf = vec![0u8; 4096];

      loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 {
          println!("Connection closed: {addr}");
          break;
        }

        let received = String::from_utf8_lossy(&buf[..n]);
        println!("[{addr}] Received: {received}");

        // Echo back
        stream.write_all(&buf[..n]).await?;
      }

      Ok(())
    })
  })
  .await?;

  Ok(())
}
