use tako::webtransport::{serve_webtransport, WebTransportSession};

async fn handle_session(session: WebTransportSession) {
  let remote = session.remote_address();
  println!("New WebTransport session from {remote}");

  // Handle bidirectional streams (echo server)
  loop {
    tokio::select! {
      // Accept bidirectional streams
      result = session.accept_bi() => {
        match result {
          Ok((mut send, mut recv)) => {
            println!("[{remote}] New bidirectional stream");
            tokio::spawn(async move {
              let mut buf = vec![0u8; 4096];
              loop {
                match recv.read(&mut buf).await {
                  Ok(Some(n)) => {
                    println!("[stream] Echoing {n} bytes");
                    if send.write_all(&buf[..n]).await.is_err() {
                      break;
                    }
                  }
                  _ => break,
                }
              }
            });
          }
          Err(e) => {
            println!("[{remote}] Connection closed: {e}");
            break;
          }
        }
      }
      // Read unreliable datagrams
      result = session.read_datagram() => {
        match result {
          Ok(data) => {
            println!("[{remote}] Datagram: {} bytes", data.len());
            // Echo datagram back
            let _ = session.send_datagram(data);
          }
          Err(e) => {
            println!("[{remote}] Datagram error: {e}");
            break;
          }
        }
      }
    }
  }

  println!("Session closed: {remote}");
}

#[tokio::main]
async fn main() {
  tracing_subscriber::fmt::init();

  println!("WebTransport echo server on [::]:4433");
  println!("Requires TLS certificates: cert.pem and key.pem");
  println!();
  println!("Generate self-signed certs for testing:");
  println!("  openssl req -x509 -newkey ec -pkeyopt ec_paramgen_curve:prime256v1 \\");
  println!("    -keyout key.pem -out cert.pem -days 365 -nodes -subj '/CN=localhost'");

  serve_webtransport("[::]:4433", "cert.pem", "key.pem", |session| {
    Box::pin(handle_session(session))
  })
  .await;
}
