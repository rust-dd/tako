#![cfg(feature = "ws")]

use std::io::Read;
use std::io::Write;
use std::net::SocketAddr;
use std::net::TcpStream;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako::ServerHandle;
use tako::router::Router;
use tako::types::Request;
use tako::ws::WebSocketUpgrade;
use tokio::sync::oneshot;

const UPGRADE: &[u8] = b"GET /ws HTTP/1.1\r\nHost: localhost\r\nUpgrade: websocket\r\n\
Connection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n\
Sec-WebSocket-Version: 13\r\n\r\n";

async fn serve(router: Router) -> ServerHandle {
  #[cfg(not(feature = "compio"))]
  {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    tako::Server::builder()
      .build()
      .try_spawn_http(listener, router)
      .unwrap()
  }
  #[cfg(feature = "compio")]
  {
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    tako::CompioServer::builder()
      .build()
      .try_spawn_http(listener, router)
      .unwrap()
  }
}

fn client(
  address: SocketAddr,
) -> (
  std::thread::JoinHandle<()>,
  oneshot::Receiver<()>,
  oneshot::Receiver<std::io::Result<Vec<u8>>>,
) {
  let (ready, ready_rx) = oneshot::channel();
  let (done, done_rx) = oneshot::channel();
  let thread = std::thread::spawn(move || {
    let result = (|| {
      let mut stream = TcpStream::connect(address)?;
      stream.set_read_timeout(Some(Duration::from_secs(2)))?;
      stream.write_all(UPGRADE)?;
      let mut head = Vec::new();
      while !head.ends_with(b"\r\n\r\n") {
        let mut byte = [0];
        stream.read_exact(&mut byte)?;
        head.push(byte[0]);
        assert!(head.len() < 8192);
      }
      assert!(head.starts_with(b"HTTP/1.1 101"));
      let _ = ready.send(());
      let mut frames = Vec::new();
      stream.read_to_end(&mut frames)?;
      Ok(frames)
    })();
    let _ = done.send(result);
  });
  (thread, ready_rx, done_rx)
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn shutdown_sends_going_away_and_closes_the_session() {
  let mut router = Router::new();
  router.get("/ws", |req: Request| async move {
    WebSocketUpgrade::new(req, |mut ws| async move {
      while let Some(Ok(_)) = ws.recv().await {}
    })
  });
  let server = serve(router).await;
  let (thread, ready, done) = client(server.local_addr().unwrap());
  ready.await.unwrap();
  server.shutdown(Duration::from_secs(1)).await;
  let frames = done.await.unwrap();
  thread.join().unwrap();
  assert_eq!(frames.unwrap(), [0x88, 2, 0x03, 0xE9]);
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn forced_shutdown_closes_a_socket_retained_by_another_task() {
  let (started, mut started_rx) = tokio::sync::mpsc::channel(1);
  let mut router = Router::new();
  router.get("/ws", move |req: Request| {
    let started = started.clone();
    async move {
      WebSocketUpgrade::new(req, |ws| async move {
        let hold = async move {
          let _socket = ws;
          std::future::pending::<()>().await;
        };
        #[cfg(not(feature = "compio"))]
        tokio::spawn(hold);
        #[cfg(feature = "compio")]
        compio::runtime::spawn(hold).detach();
        started.send(()).await.unwrap();
      })
    }
  });
  let server = serve(router).await;
  let (thread, ready, done) = client(server.local_addr().unwrap());
  ready.await.unwrap();
  started_rx.recv().await.unwrap();
  server.shutdown(Duration::ZERO).await;
  let frames = done.await.unwrap();
  thread.join().unwrap();
  let frames = frames.expect("a retained WebSocket handle must not retain a stopped socket");
  assert!(frames.is_empty() || frames == [0x88, 2, 0x03, 0xE9]);
}

struct Dropped(Arc<AtomicBool>);

impl Drop for Dropped {
  fn drop(&mut self) {
    self.0.store(true, Ordering::SeqCst);
  }
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn shutdown_returns_only_after_the_session_handler_is_dropped() {
  let dropped = Arc::new(AtomicBool::new(false));
  let (started, mut ready) = tokio::sync::mpsc::channel(1);
  let mut router = Router::new();
  let handler_dropped = dropped.clone();
  router.get("/ws", move |req: Request| {
    let guard = Dropped(handler_dropped.clone());
    let started = started.clone();
    async move {
      WebSocketUpgrade::new(req, |socket| async move {
        let _guard = guard;
        let _socket = socket;
        started.send(()).await.unwrap();
        std::future::pending::<()>().await;
      })
    }
  });
  let server = serve(router).await;
  let (thread, upgraded, done) = client(server.local_addr().unwrap());
  upgraded.await.unwrap();
  ready.recv().await.unwrap();
  server.shutdown(Duration::ZERO).await;
  assert!(dropped.load(Ordering::SeqCst));
  done.await.unwrap().unwrap();
  thread.join().unwrap();
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn a_handler_constructor_panic_closes_a_retained_socket() {
  let mut router = Router::new();
  router.get("/ws", |req: Request| async move {
    WebSocketUpgrade::new(req, |socket| -> std::future::Ready<()> {
      let hold = async move {
        let _socket = socket;
        std::future::pending::<()>().await;
      };
      #[cfg(not(feature = "compio"))]
      tokio::spawn(hold);
      #[cfg(feature = "compio")]
      compio::runtime::spawn(hold).detach();
      panic!("handler construction failed");
    })
  });
  let server = serve(router).await;
  let (thread, ready, done) = client(server.local_addr().unwrap());
  ready.await.unwrap();
  let frames = done.await.unwrap().unwrap();
  thread.join().unwrap();
  assert_eq!(frames, [0x88, 2, 0x03, 0xF3]);
  server.shutdown(Duration::from_secs(1)).await;
}
