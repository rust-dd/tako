//! Integration tests for UDP server, TCP server, and upload progress middleware.

use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

#[test]
fn progress_state_percent_normal() {
  use tako::middleware::upload_progress::ProgressState;

  let state = ProgressState {
    bytes_read: 50,
    total_bytes: Some(100),
  };
  assert_eq!(state.percent(), Some(50));
}

#[test]
fn progress_state_percent_complete() {
  use tako::middleware::upload_progress::ProgressState;

  let state = ProgressState {
    bytes_read: 100,
    total_bytes: Some(100),
  };
  assert_eq!(state.percent(), Some(100));
}

#[test]
fn progress_state_percent_zero_total() {
  use tako::middleware::upload_progress::ProgressState;

  // Edge case: 0/0 should return 100%
  let state = ProgressState {
    bytes_read: 0,
    total_bytes: Some(0),
  };
  assert_eq!(state.percent(), Some(100));
}

#[test]
fn progress_state_percent_unknown_total() {
  use tako::middleware::upload_progress::ProgressState;

  let state = ProgressState {
    bytes_read: 42,
    total_bytes: None,
  };
  assert_eq!(state.percent(), None);
}

#[tokio::test]
async fn upload_progress_callback_fires() {
  use tako::Method;
  use tako::middleware::IntoMiddleware;
  use tako::middleware::upload_progress::UploadProgress;
  use tako::responder::Responder;
  use tako::router::Router;
  use tako::types::Request;

  let callback_bytes = Arc::new(AtomicU64::new(0));
  let cb = callback_bytes.clone();

  let progress = UploadProgress::new().on_progress(move |state| {
    cb.store(state.bytes_read, Ordering::Relaxed);
  });
  let mw = progress.into_middleware();

  // Streaming progress increments only as the body is read; drain it explicitly.
  async fn handler(req: Request) -> impl Responder {
    use http_body_util::BodyExt;
    let _ = req.into_body().collect().await;
    "ok"
  }

  let mut router = Router::new();
  router
    .route(Method::POST, "/upload", handler)
    .middleware(mw);

  let payload = b"hello world upload test data";
  let req = http::Request::builder()
    .method("POST")
    .uri("/upload")
    .header("content-length", payload.len().to_string())
    .body(tako::body::TakoBody::from(payload.to_vec()))
    .unwrap();

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), 200);

  // Callback should have been called with the total bytes
  assert_eq!(callback_bytes.load(Ordering::Relaxed), payload.len() as u64);
}

#[tokio::test]
async fn upload_progress_tracker_in_extensions() {
  use tako::Method;
  use tako::middleware::IntoMiddleware;
  use tako::middleware::upload_progress::ProgressTracker;
  use tako::middleware::upload_progress::UploadProgress;
  use tako::responder::Responder;
  use tako::router::Router;
  use tako::types::Request;

  let progress = UploadProgress::new();
  let mw = progress.into_middleware();

  async fn handler(req: Request) -> impl Responder {
    use http_body_util::BodyExt;
    let tracker = req.extensions().get::<ProgressTracker>().cloned();
    // Drain the body so the streaming progress wrapper increments the counter.
    let _ = req.into_body().collect().await;
    let bytes = tracker.map(|t| t.bytes_read()).unwrap_or(0);
    format!("{bytes}")
  }

  let mut router = Router::new();
  router
    .route(Method::POST, "/upload", handler)
    .middleware(mw);

  let payload = b"test payload data 12345";
  let req = http::Request::builder()
    .method("POST")
    .uri("/upload")
    .header("content-length", payload.len().to_string())
    .body(tako::body::TakoBody::from(payload.to_vec()))
    .unwrap();

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), 200);

  use http_body_util::BodyExt;
  let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
  let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
  assert_eq!(body_str, format!("{}", payload.len()));
}

#[tokio::test]
async fn upload_progress_percent_via_tracker() {
  use tako::Method;
  use tako::middleware::IntoMiddleware;
  use tako::middleware::upload_progress::ProgressTracker;
  use tako::middleware::upload_progress::UploadProgress;
  use tako::responder::Responder;
  use tako::router::Router;
  use tako::types::Request;

  let progress = UploadProgress::new();
  let mw = progress.into_middleware();

  async fn handler(req: Request) -> impl Responder {
    use http_body_util::BodyExt;
    let tracker = req.extensions().get::<ProgressTracker>().cloned();
    // Drain the body so the streaming progress wrapper increments the counter.
    let _ = req.into_body().collect().await;
    let pct = tracker.and_then(|t| t.percent()).unwrap_or(0);
    format!("{pct}")
  }

  let mut router = Router::new();
  router
    .route(Method::POST, "/upload", handler)
    .middleware(mw);

  let payload = b"some bytes";
  let req = http::Request::builder()
    .method("POST")
    .uri("/upload")
    .header("content-length", payload.len().to_string())
    .body(tako::body::TakoBody::from(payload.to_vec()))
    .unwrap();

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), 200);

  use http_body_util::BodyExt;
  let body_bytes = resp.into_body().collect().await.unwrap().to_bytes();
  let body_str = String::from_utf8(body_bytes.to_vec()).unwrap();
  // All bytes are consumed by the time handler runs, so should be 100%
  assert_eq!(body_str, "100");
}

#[tokio::test]
async fn upload_progress_min_notify_interval() {
  use tako::Method;
  use tako::middleware::IntoMiddleware;
  use tako::middleware::upload_progress::UploadProgress;
  use tako::responder::Responder;
  use tako::router::Router;
  use tako::types::Request;

  let call_count = Arc::new(AtomicU64::new(0));
  let cc = call_count.clone();

  let progress = UploadProgress::new()
    .on_progress(move |_state| {
      cc.fetch_add(1, Ordering::Relaxed);
    })
    .min_notify_interval_bytes(1_000_000); // 1MB — well above our payload
  let mw = progress.into_middleware();

  async fn handler(_req: Request) -> impl Responder {
    "ok"
  }

  let mut router = Router::new();
  router
    .route(Method::POST, "/upload", handler)
    .middleware(mw);

  let payload = vec![0u8; 100]; // 100 bytes — well below 1MB interval
  let req = http::Request::builder()
    .method("POST")
    .uri("/upload")
    .header("content-length", payload.len().to_string())
    .body(tako::body::TakoBody::from(payload))
    .unwrap();

  let _resp = router.dispatch(req).await;

  // With a single frame body well below the interval, at most 1 callback
  // (the final notification fires if bytes != last_notified_at)
  let count = call_count.load(Ordering::Relaxed);
  assert!(count <= 1, "expected at most 1 callback, got {count}");
}

#[tokio::test]
async fn tcp_echo_server() {
  use tako::server_tcp::serve_tcp_with_shutdown;
  use tokio::io::AsyncReadExt;
  use tokio::io::AsyncWriteExt;

  let (tx, rx) = tokio::sync::oneshot::channel::<()>();

  // Find a free port
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let port = listener.local_addr().unwrap().port();
  drop(listener);

  let addr = format!("127.0.0.1:{port}");
  let addr_clone = addr.clone();

  let server = tokio::spawn(async move {
    serve_tcp_with_shutdown(
      &addr_clone,
      |mut stream, _addr| {
        Box::pin(async move {
          let mut buf = vec![0u8; 1024];
          let n = stream.read(&mut buf).await?;
          stream.write_all(&buf[..n]).await?;
          Ok(())
        })
      },
      async {
        rx.await.ok();
      },
    )
    .await
    .unwrap();
  });

  // Wait for server to start
  tokio::time::sleep(std::time::Duration::from_millis(50)).await;

  // Connect and send data
  let mut client = tokio::net::TcpStream::connect(&addr).await.unwrap();
  client.write_all(b"hello tcp").await.unwrap();

  let mut buf = vec![0u8; 1024];
  let n = client.read(&mut buf).await.unwrap();
  assert_eq!(&buf[..n], b"hello tcp");

  // Shutdown
  tx.send(()).unwrap();
  server.await.unwrap();
}

#[tokio::test]
async fn tcp_server_multiple_connections() {
  use tako::server_tcp::serve_tcp_with_shutdown;
  use tokio::io::AsyncReadExt;
  use tokio::io::AsyncWriteExt;

  let (tx, rx) = tokio::sync::oneshot::channel::<()>();

  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let port = listener.local_addr().unwrap().port();
  drop(listener);

  let addr = format!("127.0.0.1:{port}");
  let addr_clone = addr.clone();

  let server = tokio::spawn(async move {
    serve_tcp_with_shutdown(
      &addr_clone,
      |mut stream, _addr| {
        Box::pin(async move {
          let mut buf = vec![0u8; 1024];
          let n = stream.read(&mut buf).await?;
          stream.write_all(&buf[..n]).await?;
          Ok(())
        })
      },
      async {
        rx.await.ok();
      },
    )
    .await
    .unwrap();
  });

  tokio::time::sleep(std::time::Duration::from_millis(50)).await;

  // Spawn multiple concurrent clients
  let mut handles = Vec::new();
  for i in 0..5u8 {
    let addr = addr.clone();
    handles.push(tokio::spawn(async move {
      let mut client = tokio::net::TcpStream::connect(&addr).await.unwrap();
      let msg = format!("msg-{i}");
      client.write_all(msg.as_bytes()).await.unwrap();

      let mut buf = vec![0u8; 64];
      let n = client.read(&mut buf).await.unwrap();
      assert_eq!(&buf[..n], msg.as_bytes());
    }));
  }

  for h in handles {
    h.await.unwrap();
  }

  tx.send(()).unwrap();
  server.await.unwrap();
}

#[tokio::test]
async fn udp_echo_server() {
  use tako::server_udp::serve_udp_with_shutdown;

  let (tx, rx) = tokio::sync::oneshot::channel::<()>();

  // Find a free port
  let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
  let port = sock.local_addr().unwrap().port();
  drop(sock);

  let addr = format!("127.0.0.1:{port}");
  let addr_clone = addr.clone();

  let server = tokio::spawn(async move {
    serve_udp_with_shutdown(
      &addr_clone,
      |data, peer, socket| {
        Box::pin(async move {
          let _ = socket.send_to(&data, peer).await;
        })
      },
      async {
        rx.await.ok();
      },
    )
    .await
    .unwrap();
  });

  tokio::time::sleep(std::time::Duration::from_millis(50)).await;

  // Send datagram
  let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
  client.send_to(b"hello udp", &addr).await.unwrap();

  let mut buf = vec![0u8; 1024];
  let (n, _) = client.recv_from(&mut buf).await.unwrap();
  assert_eq!(&buf[..n], b"hello udp");

  // Shutdown
  tx.send(()).unwrap();
  server.await.unwrap();
}

#[tokio::test]
async fn udp_multiple_datagrams() {
  use tako::server_udp::serve_udp_with_shutdown;

  let (tx, rx) = tokio::sync::oneshot::channel::<()>();

  let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
  let port = sock.local_addr().unwrap().port();
  drop(sock);

  let addr = format!("127.0.0.1:{port}");
  let addr_clone = addr.clone();

  let server = tokio::spawn(async move {
    serve_udp_with_shutdown(
      &addr_clone,
      |data, peer, socket| {
        Box::pin(async move {
          // Echo with prefix
          let mut response = b"echo:".to_vec();
          response.extend_from_slice(&data);
          let _ = socket.send_to(&response, peer).await;
        })
      },
      async {
        rx.await.ok();
      },
    )
    .await
    .unwrap();
  });

  tokio::time::sleep(std::time::Duration::from_millis(50)).await;

  let client = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();

  for i in 0..3 {
    let msg = format!("packet-{i}");
    client.send_to(msg.as_bytes(), &addr).await.unwrap();

    let mut buf = vec![0u8; 1024];
    let (n, _) = client.recv_from(&mut buf).await.unwrap();
    let expected = format!("echo:{msg}");
    assert_eq!(&buf[..n], expected.as_bytes());
  }

  tx.send(()).unwrap();
  server.await.unwrap();
}
