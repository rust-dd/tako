//! Local HTTP/1 keepalive/pipelining control with separate client and server runtimes.
//! Uses the standard Tokio server; this is not the Arena per-thread server profile.

#[cfg(not(feature = "compio"))]
mod tokio_benchmark {
  use std::time::Duration;
  use std::time::Instant;

  use bytes::Buf;
  use bytes::Bytes;
  use bytes::BytesMut;
  use tako_rs_core::router::Router;
  use tako_rs_server::Server;
  use tokio::io::AsyncReadExt;
  use tokio::io::AsyncWriteExt;
  use tokio::net::TcpListener;
  use tokio::net::TcpStream;
  use tokio::sync::oneshot;

  const REQUEST: &[u8] = b"GET /baseline HTTP/1.1\r\nHost: localhost\r\n\r\n";

  struct Client {
    socket: TcpStream,
    received: BytesMut,
  }

  impl Client {
    async fn read_more(&mut self) {
      assert!(self.received.len() < 64 * 1024, "oversized response header");
      let received = tokio::time::timeout(
        Duration::from_secs(30),
        self.socket.read_buf(&mut self.received),
      )
      .await
      .expect("response read timed out")
      .unwrap();
      assert!(
        received > 0,
        "connection closed before all responses arrived"
      );
    }

    async fn receive_response(&mut self) {
      let header_end = loop {
        if let Some(end) = self
          .received
          .windows(4)
          .position(|bytes| bytes == b"\r\n\r\n")
        {
          break end + 4;
        }
        self.read_more().await;
      };
      let header = std::str::from_utf8(&self.received[..header_end]).unwrap();
      let mut lines = header.split("\r\n");
      let mut status = lines.next().unwrap().split_whitespace();
      assert_eq!(status.next(), Some("HTTP/1.1"));
      assert_eq!(status.next(), Some("200"));
      let mut content_length = None;
      for line in lines.take_while(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').expect("malformed response header");
        assert!(
          !name.eq_ignore_ascii_case("transfer-encoding"),
          "the buffered baseline response must have a content length"
        );
        if name.eq_ignore_ascii_case("content-length") {
          assert!(content_length.is_none(), "duplicate content-length");
          content_length = Some(value.trim().parse::<usize>().unwrap());
        }
      }
      assert_eq!(content_length, Some(2));
      let end = header_end + 2;
      while self.received.len() < end {
        self.read_more().await;
      }
      assert_eq!(&self.received[header_end..end], b"ok");
      self.received.advance(end);
    }
  }

  fn setting(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().expect("positive integer"))
  }

  pub(super) fn benchmark() {
    let connections = setting("CONNECTIONS", 64);
    let pipeline = setting("PIPELINE", 16);
    let requests = setting("REQUESTS", 8192);
    let rounds = setting("ROUNDS", 5);
    let workers = setting("WORKERS", 4);
    assert!(connections > 0 && pipeline > 0 && requests > 0 && rounds > 0 && workers > 0);
    let (address_tx, address_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let server_thread = std::thread::spawn(move || {
      tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
          let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
          let address = listener.local_addr().unwrap();
          let mut router = Router::new();
          router.get("/baseline", || async { "ok" });
          let server = Server::builder().build().spawn_http(listener, router);
          address_tx.send(address).unwrap();
          stop_rx.await.unwrap();
          server.shutdown(Duration::from_secs(5)).await;
        });
    });
    let address = address_rx.recv().unwrap();
    tokio::runtime::Builder::new_multi_thread()
      .worker_threads(workers)
      .enable_all()
      .build()
      .unwrap()
      .block_on(async {
        let mut clients = Vec::with_capacity(connections);
        for _ in 0..connections {
          let socket = TcpStream::connect(address).await.unwrap();
          socket.set_nodelay(true).unwrap();
          clients.push(Client {
            socket,
            received: BytesMut::with_capacity(16 * 1024),
          });
        }
        let batch = Bytes::from(REQUEST.repeat(pipeline));
        run(&mut clients, &batch, pipeline, (requests / 4).max(1)).await;
        println!(
          "local_http1_control connections={connections} pipeline={pipeline} \
           requests/conn={requests} workers={workers}"
        );
        for round in 1..=rounds {
          let start = Instant::now();
          run(&mut clients, &batch, pipeline, requests).await;
          let elapsed = start.elapsed();
          let rps = (connections * requests) as f64 / elapsed.as_secs_f64();
          println!(
            "round={round} rps={rps:.0} elapsed_ms={:.1}",
            elapsed.as_secs_f64() * 1000.0
          );
        }
        drop(clients);
      });
    stop_tx.send(()).unwrap();
    server_thread.join().unwrap();
  }

  async fn run(clients: &mut Vec<Client>, batch: &Bytes, pipeline: usize, requests: usize) {
    let mut tasks = tokio::task::JoinSet::new();
    for mut client in clients.drain(..) {
      let batch = batch.clone();
      tasks.spawn(async move {
        let mut remaining = requests;
        while remaining > 0 {
          let count = remaining.min(pipeline);
          client
            .socket
            .write_all(&batch[..count * REQUEST.len()])
            .await
            .unwrap();
          for _ in 0..count {
            client.receive_response().await;
          }
          remaining -= count;
        }
        assert!(
          client.received.is_empty(),
          "unexpected extra response bytes"
        );
        client
      });
    }
    while let Some(result) = tasks.join_next().await {
      clients.push(result.unwrap());
    }
  }
}

fn main() {
  #[cfg(not(feature = "compio"))]
  tokio_benchmark::benchmark();
  #[cfg(feature = "compio")]
  eprintln!("The HTTP/1 control uses Tokio; run without the compio feature.");
}
