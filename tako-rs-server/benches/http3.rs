//! Loopback HTTP/3 throughput with separate client and server runtimes.

#[cfg(not(feature = "compio"))]
mod tokio_benchmark {
  use std::sync::Arc;
  use std::time::Duration;
  use std::time::Instant;

  use bytes::Buf;
  use bytes::Bytes;
  use futures_util::StreamExt;
  use futures_util::stream::FuturesUnordered;
  use h3::client::SendRequest;
  use tako_rs_core::router::Router;
  use tako_rs_server::Server;
  use tako_rs_server::TlsCert;
  use tokio::sync::oneshot;

  fn setting(name: &str, default: usize) -> usize {
    std::env::var(name).map_or(default, |value| value.parse().expect("positive integer"))
  }

  pub(super) fn benchmark() {
    let connections = setting("CONNECTIONS", 32);
    let streams = setting("STREAMS", 32);
    let requests = setting("REQUESTS", 2048);
    let rounds = setting("ROUNDS", 5);
    let workers = setting("WORKERS", 4);
    assert!(connections > 0 && streams > 0 && requests >= streams && rounds > 0 && workers > 0);
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    let identity = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
    let certificate = identity.cert.der().clone();
    let key = rustls::pki_types::PrivatePkcs8KeyDer::from(identity.signing_key.serialize_der());
    let server_certificate = certificate.clone();
    let (address_tx, address_rx) = std::sync::mpsc::channel();
    let (stop_tx, stop_rx) = oneshot::channel();
    let server_thread = std::thread::spawn(move || {
      tokio::runtime::Builder::new_multi_thread()
        .worker_threads(workers)
        .enable_all()
        .build()
        .unwrap()
        .block_on(async move {
          let mut router = Router::new();
          router.get("/baseline", || async { "ok" });
          let server = Server::builder()
            .tls(TlsCert::der(vec![server_certificate], key.into()))
            .build()
            .try_spawn_h3("127.0.0.1:0", router)
            .unwrap();
          address_tx.send(server.local_addr().unwrap()).unwrap();
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
        let mut roots = rustls::RootCertStore::empty();
        roots.add(certificate).unwrap();
        let mut tls = rustls::ClientConfig::builder()
          .with_root_certificates(roots)
          .with_no_client_auth();
        tls.alpn_protocols = vec![b"h3".to_vec()];
        let quic = quinn::crypto::rustls::QuicClientConfig::try_from(tls).unwrap();
        let mut endpoint = quinn::Endpoint::client("127.0.0.1:0".parse().unwrap()).unwrap();
        endpoint.set_default_client_config(quinn::ClientConfig::new(Arc::new(quic)));
        let mut clients = Vec::with_capacity(connections);
        let mut drivers = Vec::with_capacity(connections);
        for _ in 0..connections {
          let connection = endpoint
            .connect(address, "localhost")
            .unwrap()
            .await
            .unwrap();
          let (mut driver, client) = h3::client::new(h3_quinn::Connection::new(connection))
            .await
            .unwrap();
          clients.push(client);
          drivers.push(tokio::spawn(async move {
            std::future::poll_fn(|cx| driver.poll_close(cx)).await
          }));
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
        endpoint.close(0u32.into(), b"benchmark finished");
        for driver in drivers {
          driver.await.unwrap();
        }
        endpoint.wait_idle().await;
      });
    stop_tx.send(()).unwrap();
    server_thread.join().unwrap();
  }

  async fn run(
    clients: &[SendRequest<h3_quinn::OpenStreams, Bytes>],
    streams: usize,
    requests: usize,
  ) {
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
                .uri("https://localhost/baseline")
                .body(())
                .unwrap();
              let mut stream = client.send_request(request).await.unwrap();
              stream.finish().await.unwrap();
              let response = stream.recv_response().await.unwrap();
              assert_eq!(response.status(), 200);
              assert_eq!(response.version(), http::Version::HTTP_3);
              let mut body = [0; 2];
              let mut received = 0;
              while let Some(mut data) = stream.recv_data().await.unwrap() {
                let len = data.remaining();
                data.copy_to_slice(&mut body[received..received + len]);
                received += len;
              }
              assert_eq!(received, body.len());
              assert_eq!(&body, b"ok");
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
  eprintln!("The HTTP/3 benchmark uses Tokio; run without the compio feature.");
}
