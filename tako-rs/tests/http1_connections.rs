#![cfg(not(feature = "compio"))]

use std::convert::Infallible;
use std::future::Future;
use std::net::SocketAddr;
use std::time::Duration;

use tako::Bytes;
use tako::ServerConfig;
use tako::body::TakoBody;
use tako::extractors::FromRequest;
use tako::extractors::connect_info::ConnectInfo;
use tako::extractors::ipaddr::IpAddr;
use tako::extractors::json::Json;
use tako::extractors::matched_path::MatchedPath;
use tako::extractors::path::Path;
use tako::extractors::query::Query;
use tako::extractors::state::State;
use tako::router::Router;
use tako::types::Request;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;

const HEADER_DEADLINE: Duration = Duration::from_millis(50);

#[derive(Clone)]
struct Marker;

struct Marked(bool);

impl<'a> FromRequest<'a> for Marked {
  type Error = Infallible;

  fn from_request(
    req: &'a mut Request,
  ) -> impl Future<Output = Result<Self, Self::Error>> + Send + 'a {
    std::future::ready(Ok(Marked(req.extensions().get::<Marker>().is_some())))
  }
}

async fn describe(ConnectInfo(peer): ConnectInfo<SocketAddr>, Marked(marked): Marked) -> String {
  format!("marked={marked} peer={}", peer.port())
}

#[derive(Clone)]
struct Greeting(&'static str);

fn router() -> Router {
  let mut router = Router::new();
  router.with_state(Greeting("hi"));
  router.body_limit(64);
  router.get(
    "/x/conn",
    |ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { peer.port().to_string() },
  );
  router.get("/x/ip", |ip: IpAddr| async move { ip.to_string() });
  router.get("/x/state", |State(greeting): State<Greeting>| async move {
    greeting.0
  });
  router.get("/x/path/{id}", |Path(id): Path<String>| async move { id });
  router.get(
    "/x/query",
    |Query(query): Query<std::collections::HashMap<String, String>>| async move {
      query.get("a").cloned().unwrap_or_default()
    },
  );
  router.get("/x/matched/{id}", |path: MatchedPath| async move {
    path.as_str().to_owned()
  });
  router.post(
    "/x/json",
    |Json(value): Json<serde_json::Value>| async move { value["a"].to_string() },
  );
  router
    .get("/", describe)
    .middleware(|mut req: Request, next| async move {
      if req.headers().contains_key("x-mark") {
        req.extensions_mut().insert(Marker);
      }
      next.run(req).await
    });
  router.get("/plain", describe);
  router.get("/stream", || async {
    let chunks = futures_util::stream::unfold(0, |index| async move {
      if index == 5 {
        return None;
      }
      tokio::time::sleep(Duration::from_millis(40)).await;
      Some((
        Ok::<_, Infallible>(Bytes::from(format!("chunk{index};"))),
        index + 1,
      ))
    });
    TakoBody::from_stream(chunks)
  });
  router
}

/// Runs `test` against the multi-threaded server and, on Unix with the
/// `per-thread` feature, against a per-thread worker.
async fn on_each_server<F, Fut>(test: F)
where
  F: Fn(SocketAddr) -> Fut,
  Fut: Future<Output = ()>,
{
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = tako::Server::builder()
    .config(ServerConfig {
      header_read_timeout: Some(HEADER_DEADLINE),
      ..ServerConfig::default()
    })
    .build()
    .spawn_http(listener, router());
  test(address).await;
  handle.shutdown(Duration::from_secs(1)).await;

  #[cfg(all(unix, feature = "per-thread"))]
  {
    let address = std::net::TcpListener::bind("127.0.0.1:0")
      .unwrap()
      .local_addr()
      .unwrap();
    let (threads, shutdown) = tako::spawn_per_thread(
      &address.to_string(),
      router(),
      tako::PerThreadConfig {
        workers: 1,
        pin_to_core: false,
        header_read_timeout: Some(HEADER_DEADLINE),
        ..tako::PerThreadConfig::default()
      },
    )
    .unwrap();
    tokio::time::timeout(Duration::from_secs(2), shutdown.wait_for_bind_outcome(1))
      .await
      .unwrap()
      .unwrap();
    test(address).await;
    shutdown.trigger();
    tokio::task::spawn_blocking(move || {
      for thread in threads {
        thread.join().unwrap();
      }
    })
    .await
    .unwrap();
  }
}

async fn get(stream: &mut TcpStream, path: &str, headers: &str) -> String {
  send(
    stream,
    &format!("GET {path} HTTP/1.1\r\nHost: localhost\r\n{headers}\r\n"),
  )
  .await
}

async fn post_json(stream: &mut TcpStream, path: &str, body: &str) -> String {
  let request = format!(
    "POST {path} HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
    body.len()
  );
  send(stream, &request).await
}

async fn send(stream: &mut TcpStream, request: &str) -> String {
  stream.write_all(request.as_bytes()).await.unwrap();
  let mut received = Vec::new();
  let mut chunk = [0; 4096];
  loop {
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut chunk))
      .await
      .expect("response must arrive")
      .unwrap();
    assert!(
      read > 0,
      "closed early: {}",
      String::from_utf8_lossy(&received)
    );
    received.extend_from_slice(&chunk[..read]);
    if let Some(body) = complete_body(&received) {
      return body;
    }
  }
}

fn complete_body(received: &[u8]) -> Option<String> {
  let text = String::from_utf8_lossy(received);
  let (head, body) = text.split_once("\r\n\r\n")?;
  let head = head.to_ascii_lowercase();
  if let Some(length) = head
    .lines()
    .find_map(|line| line.strip_prefix("content-length: "))
  {
    let length: usize = length.trim().parse().unwrap();
    return (body.len() >= length).then(|| body[..length].to_owned());
  }
  body.ends_with("0\r\n\r\n").then(|| body.to_owned())
}

#[tokio::test]
async fn streamed_response_outlives_the_header_deadline() {
  on_each_server(|address| async move {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let body = get(&mut stream, "/stream", "").await;
    for index in 0..5 {
      assert!(body.contains(&format!("chunk{index};")), "{body}");
    }
  })
  .await;
}

#[tokio::test]
async fn request_entries_do_not_leak_into_later_requests() {
  on_each_server(|address| async move {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let port = stream.local_addr().unwrap().port();
    assert_eq!(
      get(&mut stream, "/", "x-mark: 1\r\n").await,
      format!("marked=true peer={port}")
    );
    assert_eq!(
      get(&mut stream, "/plain", "").await,
      format!("marked=false peer={port}")
    );
    assert_eq!(
      get(&mut stream, "/", "").await,
      format!("marked=false peer={port}")
    );
  })
  .await;
}

#[tokio::test]
async fn idle_keep_alive_connection_closes_after_the_header_deadline() {
  on_each_server(|address| async move {
    let mut stream = TcpStream::connect(address).await.unwrap();
    get(&mut stream, "/plain", "").await;
    let mut rest = Vec::new();
    tokio::time::timeout(Duration::from_secs(1), stream.read_to_end(&mut rest))
      .await
      .expect("idle connection must close")
      .unwrap();
    assert!(rest.is_empty());
  })
  .await;
}

#[tokio::test]
async fn built_in_extractors_get_their_entries_without_middleware() {
  on_each_server(|address| async move {
    let mut stream = TcpStream::connect(address).await.unwrap();
    let port = stream.local_addr().unwrap().port();
    assert_eq!(get(&mut stream, "/x/conn", "").await, port.to_string());
    assert_eq!(get(&mut stream, "/x/path/7", "").await, "7");
    assert_eq!(get(&mut stream, "/x/ip", "").await, "127.0.0.1");
    assert_eq!(get(&mut stream, "/x/state", "").await, "hi");
    assert_eq!(get(&mut stream, "/x/matched/7", "").await, "/x/matched/{id}");
    assert_eq!(get(&mut stream, "/x/query?a=1", "").await, "1");
    assert_eq!(post_json(&mut stream, "/x/json", r#"{"a":2}"#).await, "2");
    assert_eq!(get(&mut stream, "/x/conn", "").await, port.to_string());

    let mut stream = TcpStream::connect(address).await.unwrap();
    let oversized = format!(r#"{{"a":"{}"}}"#, "x".repeat(100));
    let request = format!(
      "POST /x/json HTTP/1.1\r\nHost: localhost\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{oversized}",
      oversized.len()
    );
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut head = [0; 64];
    let read = tokio::time::timeout(Duration::from_secs(2), stream.read(&mut head))
      .await
      .unwrap()
      .unwrap();
    assert!(
      String::from_utf8_lossy(&head[..read]).starts_with("HTTP/1.1 413"),
      "{}",
      String::from_utf8_lossy(&head[..read])
    );
  })
  .await;
}
