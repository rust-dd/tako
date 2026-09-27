#![cfg(feature = "signals")]

use std::sync::Arc;
use std::time::Duration;

use tako::router::Router;
use tako::signals::app_signals;
use tako::signals::ids;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn lifecycle_events_bracket_the_listener_and_release_router_state() {
  let state = Arc::new(());
  let mut router = Router::new();
  router.with_state(state.clone());
  router.get("/", || async { "ok" });
  let mut started = app_signals().subscribe(ids::SERVER_STARTED);
  let mut stopped = app_signals().subscribe(ids::SERVER_STOPPED);
  #[cfg(not(feature = "compio"))]
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  #[cfg(feature = "compio")]
  let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  #[cfg(not(feature = "compio"))]
  let server = tako::Server::builder().build();
  #[cfg(feature = "compio")]
  let server = tako::CompioServer::builder().build();
  let handle = server.try_spawn_http(listener, router).unwrap();
  let event = started.recv().await.unwrap();
  assert_eq!(event.metadata["addr"], address.to_string());
  assert!(stopped.try_recv().is_err());
  handle.shutdown(Duration::from_secs(1)).await;
  let event = stopped.try_recv().unwrap();
  assert_eq!(event.metadata["addr"], address.to_string());
  assert_eq!(event.metadata["transport"], "tcp");
  assert_eq!(Arc::strong_count(&state), 1);
}
