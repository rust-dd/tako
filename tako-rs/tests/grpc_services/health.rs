use std::net::SocketAddr;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tako::grpc::health::HealthRegistry;
use tako::grpc::health::ServingStatus;
use tonic::Code;
use tonic_prost::ProstCodec;

use super::client;
use super::wire::HealthListRequest;
use super::wire::HealthListResponse;
use super::wire::HealthRequest;
use super::wire::HealthResponse;

fn request(service: &str) -> tonic::Request<HealthRequest> {
  tonic::Request::new(HealthRequest {
    service: service.into(),
  })
}

pub async fn check(address: SocketAddr, registry: &HealthRegistry) {
  let mut grpc = client(address).await;
  let codec = ProstCodec::<HealthRequest, HealthResponse>::new;
  for (service, expected) in [("", Ok(1)), ("missing", Err(Code::NotFound))] {
    grpc.ready().await.unwrap();
    let response = grpc
      .unary(
        request(service),
        "/grpc.health.v1.Health/Check".parse().unwrap(),
        codec(),
      )
      .await
      .map(|response| response.into_inner().status)
      .map_err(|status| status.code());
    assert_eq!(response, expected);
  }
  registry.clone().set_status("", ServingStatus::NotServing);
  grpc.ready().await.unwrap();
  let response = grpc
    .unary(
      request(""),
      "/grpc.health.v1.Health/Check".parse().unwrap(),
      codec(),
    )
    .await
    .unwrap();
  assert_eq!(response.into_inner().status, 2);
  grpc.ready().await.unwrap();
  let response = grpc
    .unary(
      tonic::Request::new(HealthListRequest {}),
      "/grpc.health.v1.Health/List".parse().unwrap(),
      ProstCodec::<HealthListRequest, HealthListResponse>::new(),
    )
    .await
    .unwrap()
    .into_inner();
  assert_eq!(response.statuses.len(), 1);
  assert_eq!(response.statuses[""].status, 2);
}

pub async fn watch(address: SocketAddr, registry: &HealthRegistry, dropped: &AtomicUsize) {
  let mut grpc = client(address).await;
  grpc.ready().await.unwrap();
  let mut watch = grpc
    .server_streaming(
      request("later"),
      "/grpc.health.v1.Health/Watch".parse().unwrap(),
      ProstCodec::<HealthRequest, HealthResponse>::new(),
    )
    .await
    .unwrap()
    .into_inner();
  assert_eq!(watch.message().await.unwrap().unwrap().status, 3);
  for (status, code) in [
    (ServingStatus::Serving, 1),
    (ServingStatus::NotServing, 2),
    (ServingStatus::Unknown, 0),
  ] {
    registry.set_status("later", status);
    assert_eq!(watch.message().await.unwrap().unwrap().status, code);
  }
  registry.set_status("later", ServingStatus::Unknown);
  registry.set_status("unrelated", ServingStatus::Serving);
  assert!(
    tokio::time::timeout(Duration::from_millis(50), watch.message())
      .await
      .is_err()
  );
  let previous = dropped.load(Ordering::SeqCst);
  drop(watch);
  super::cancellation::wait(dropped, previous).await;
  registry.set_status("later", ServingStatus::Serving);
}

pub async fn deadline(address: SocketAddr) {
  let mut grpc = client(address).await;
  let mut request = request("never");
  request.set_timeout(Duration::from_millis(80));
  grpc.ready().await.unwrap();
  let mut watch = grpc
    .server_streaming(
      request,
      "/grpc.health.v1.Health/Watch".parse().unwrap(),
      ProstCodec::<HealthRequest, HealthResponse>::new(),
    )
    .await
    .unwrap()
    .into_inner();
  assert_eq!(watch.message().await.unwrap().unwrap().status, 3);
  assert_eq!(
    watch.message().await.unwrap_err().code(),
    Code::DeadlineExceeded
  );
}
