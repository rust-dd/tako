#![cfg(all(unix, feature = "per-thread", feature = "plugins"))]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako::PerThreadConfig;
use tako::plugins::TakoPlugin;
use tako::router::Router;
use tako::spawn_per_thread;
use tokio::io::AsyncReadExt;
use tokio::io::AsyncWriteExt;

#[derive(Clone)]
struct StartupPlugin(Arc<AtomicUsize>);

impl TakoPlugin for StartupPlugin {
  fn name(&self) -> &'static str {
    "startup-test"
  }
  fn setup(&self, router: &Router) -> anyhow::Result<()> {
    self.0.fetch_add(1, Ordering::SeqCst);
    router.middleware(|request, next| async move {
      let mut response = next.run(request).await;
      response
        .headers_mut()
        .insert("x-plugin", "ready".parse().unwrap());
      response
    });
    Ok(())
  }
}

#[tokio::test]
async fn per_thread_initializes_plugins_once_and_drains_idle_connections() {
  let calls = Arc::new(AtomicUsize::new(0));
  let state = Arc::new(());
  let mut router = Router::new();
  router.with_state(state.clone());
  router.plugin(StartupPlugin(calls.clone()));
  router.plugin(tako::plugins::rate_limiter::RateLimiterBuilder::new().build());
  router.timeout(Duration::from_secs(1));
  router.get("/", || async { "ok" });
  let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
  let address = listener.local_addr().unwrap();
  drop(listener);
  let (threads, shutdown) = spawn_per_thread(
    &address.to_string(),
    router,
    PerThreadConfig {
      workers: 2,
      pin_to_core: false,
      drain_timeout: Duration::from_secs(5),
      max_connections: Some(2),
      header_read_timeout: Some(Duration::from_millis(100)),
      ..PerThreadConfig::default()
    },
  )
  .unwrap();
  tokio::time::timeout(Duration::from_secs(2), shutdown.wait_for_bind_outcome(2))
    .await
    .unwrap()
    .unwrap();
  let mut stream = tokio::net::TcpStream::connect(address).await.unwrap();
  stream
    .write_all(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n")
    .await
    .unwrap();
  let mut response = [0; 1024];
  let read = tokio::time::timeout(Duration::from_secs(1), stream.read(&mut response))
    .await
    .unwrap()
    .unwrap();
  assert!(String::from_utf8_lossy(&response[..read]).contains("x-plugin: ready"));
  let mut partial = tokio::net::TcpStream::connect(address).await.unwrap();
  partial.write_all(b"GET / HTTP/1.1\r\nHost:").await.unwrap();
  let mut rejected = Vec::new();
  tokio::time::timeout(Duration::from_secs(1), partial.read_to_end(&mut rejected))
    .await
    .unwrap()
    .unwrap();
  shutdown.trigger();
  tokio::time::timeout(
    Duration::from_secs(1),
    tokio::task::spawn_blocking(move || {
      for thread in threads {
        thread.join().unwrap();
      }
    }),
  )
  .await
  .unwrap()
  .unwrap();
  assert_eq!(calls.load(Ordering::SeqCst), 1);
  assert_eq!(Arc::strong_count(&state), 1);
}
