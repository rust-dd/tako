#![cfg(all(feature = "grpc", feature = "http2"))]

//! Standard services exercised through independent protobuf messages and a tonic client.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::time::Duration;

use tako::grpc::health::HealthRegistry;
use tako::grpc::health::ServingStatus;
use tako::router::Router;
use tonic::transport::Channel;

#[path = "grpc_services/cancellation.rs"]
mod cancellation;
#[path = "grpc_services/health.rs"]
mod health;
#[path = "grpc_services/reflection.rs"]
mod reflection;
#[path = "grpc_services/wire.rs"]
mod wire;

async fn client(address: SocketAddr) -> tonic::client::Grpc<Channel> {
  let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
    .unwrap()
    .connect()
    .await
    .unwrap();
  tonic::client::Grpc::new(channel)
}

fn router(health: &HealthRegistry, dropped: &Arc<AtomicUsize>) -> Router {
  let mut router = Router::new();
  cancellation::monitor(&router, dropped);
  health.set_status("", ServingStatus::Serving);
  health.register(&mut router);
  reflection::registry().register(&mut router);
  router
}

async fn exercise(address: SocketAddr, health: HealthRegistry, dropped: Arc<AtomicUsize>) {
  health::check(address, &health).await;
  health::watch(address, &health, &dropped).await;
  health::deadline(address).await;
  reflection::lookups(address).await;
  reflection::errors(address).await;
  reflection::alpha(address).await;
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn standard_grpc_services_on_tokio() {
  let health = HealthRegistry::new();
  let dropped = Arc::new(AtomicUsize::new(0));
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = tako::Server::builder()
    .build()
    .try_spawn_h2c(listener, router(&health, &dropped))
    .unwrap();
  tokio::time::timeout(Duration::from_secs(15), exercise(address, health, dropped))
    .await
    .expect("standard gRPC services timed out");
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "compio")]
#[test]
fn standard_grpc_services_on_compio() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let health = HealthRegistry::new();
    let dropped = Arc::new(AtomicUsize::new(0));
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tako::CompioServer::builder()
      .build()
      .try_spawn_h2c(listener, router(&health, &dropped))
      .unwrap();
    let (done, finished) = tokio::sync::oneshot::channel();
    let client = std::thread::spawn(move || {
      tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(exercise(address, health, dropped));
      let _ = done.send(());
    });
    compio::time::timeout(Duration::from_secs(15), finished)
      .await
      .expect("standard gRPC services timed out")
      .ok();
    if let Err(panic) = client.join() {
      std::panic::resume_unwind(panic);
    }
    handle.shutdown(Duration::from_secs(1)).await;
  });
}
