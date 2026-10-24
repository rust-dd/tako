//! Loopback HTTP/2 throughput with separate client and server runtimes.

#[cfg(not(feature = "compio"))]
mod tokio_benchmark {

  use std::net::SocketAddr;
  use std::time::Duration;
  use std::time::Instant;

  use futures_util::StreamExt;
  use futures_util::stream::FuturesUnordered;
  use http_body_util::BodyExt;
  use hyper::client::conn::http2::SendRequest;
  use hyper_util::rt::TokioExecutor;
  use hyper_util::rt::TokioIo;
  use tako_rs_core::body::TakoBody;
  use tako_rs_core::router::Router;
  use tako_rs_server::Server;
  use tokio::net::TcpListener;
  use tokio::net::TcpStream;
  use tokio::sync::oneshot;

  fn setting(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().expect("positive integer"))
  }

  pub(super) fn benchmark() {
    let connections = setting("CONNECTIONS", 64);
    let streams = setting("STREAMS", 32);
    let requests = setting("REQUESTS", 4096);
    let rounds = setting("ROUNDS", 5);
    let workers = setting("WORKERS", 4);
    assert!(connections > 0 && streams > 0 && requests >= streams && rounds > 0 && workers > 0);
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
          let server = Server::builder().build().spawn_h2c(listener, router);
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
        let mut drivers = Vec::with_capacity(connections);
        for _ in 0..connections {
          let (client, driver) = connect(address).await;
          clients.push(client);
          drivers.push(driver);
        }
        run(&clients, streams, requests / 4).await;
        println!(
          "connections={connections} streams={streams} requests/conn={requests} workers={workers}"
        );
        for round in 1..=rounds {
          let start = Instant::now();
          run(&clients, streams, requests).await;
          let elapsed = start.elapsed();
          let rps = (connections * requests) as f64 / elapsed.as_secs_f64();
          println!(
            "round={round} rps={rps:.0} elapsed_ms={:.1}",
            elapsed.as_secs_f64() * 1000.0
          );
        }
        drop(clients);
        for driver in drivers {
          driver.await.unwrap().unwrap();
        }
      });
    stop_tx.send(()).unwrap();
    server_thread.join().unwrap();
  }

  async fn connect(
    address: SocketAddr,
  ) -> (
    SendRequest<TakoBody>,
    tokio::task::JoinHandle<Result<(), hyper::Error>>,
  ) {
    let socket = TcpStream::connect(address).await.unwrap();
    socket.set_nodelay(true).unwrap();
    let (sender, connection) =
      hyper::client::conn::http2::handshake(TokioExecutor::new(), TokioIo::new(socket))
        .await
        .unwrap();
    (sender, tokio::spawn(connection))
  }

  async fn run(clients: &[SendRequest<TakoBody>], streams: usize, requests: usize) {
    let mut tasks = tokio::task::JoinSet::new();
    for client in clients {
      let client = client.clone();
      tasks.spawn(async move {
        let mut pending = FuturesUnordered::new();
        let mut sent = 0;
        while sent < requests || !pending.is_empty() {
          while sent < requests && pending.len() < streams {
            let mut client = client.clone();
            pending.push(async move {
              let request = http::Request::builder()
                .uri("http://localhost/baseline")
                .body(TakoBody::empty())
                .unwrap();
              let response = client.send_request(request).await.unwrap();
              assert_eq!(response.status(), 200);
              assert_eq!(response.version(), http::Version::HTTP_2);
              assert_eq!(
                response.into_body().collect().await.unwrap().to_bytes(),
                "ok"
              );
            });
            sent += 1;
          }
          pending.next().await;
        }
      });
    }
    while let Some(result) = tasks.join_next().await {
      result.unwrap();
    }
  }
}

fn main() {
  #[cfg(not(feature = "compio"))]
  tokio_benchmark::benchmark();
  #[cfg(feature = "compio")]
  eprintln!("The HTTP/2 benchmark uses Tokio; run without the compio feature.");
}
