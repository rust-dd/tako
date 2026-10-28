#![cfg(feature = "grpc")]

use futures_util::StreamExt;
use prost::Message;
use prost_types::FileDescriptorProto;
use prost_types::ServiceDescriptorProto;
use tako::body::TakoBody;
use tako::grpc::health::HealthRegistry;
use tako::grpc::health::ServingStatus;
use tako::grpc::reflection::ReflectionRegistry;
use tako::router::Router;

#[test]
fn descriptor_sets_preserve_unknown_custom_options() {
  let registry = ReflectionRegistry::new();
  let file = b"\x0a\x07a.proto\x42\x03\xa0\x06\x07";
  let set = [b"\x0a\x0e".as_slice(), file].concat();
  registry.register_encoded_file_descriptor_set(&set).unwrap();
  assert_eq!(registry.file_by_filename("a.proto").unwrap(), file);
}

#[test]
fn invalid_descriptor_sets_do_not_partially_update_the_registry() {
  for invalid in [b"\x0a\x00".as_slice(), b"\x0a\xff".as_slice()] {
    let registry = ReflectionRegistry::new();
    let bytes = [b"\x0a\x09\x0a\x07a.proto".as_slice(), invalid].concat();
    assert!(
      registry
        .register_encoded_file_descriptor_set(&bytes)
        .is_err()
    );
    assert!(registry.file_by_filename("a.proto").is_none());
  }
}

#[test]
fn replacing_a_file_removes_its_stale_symbols_and_services() {
  let registry = ReflectionRegistry::new();
  for name in ["Old", "New"] {
    let file = FileDescriptorProto {
      name: Some("api.proto".into()),
      package: Some("test".into()),
      service: vec![ServiceDescriptorProto {
        name: Some(name.into()),
        ..Default::default()
      }],
      ..Default::default()
    };
    registry.clone().add_file("api.proto", file.encode_to_vec());
  }
  assert_eq!(registry.list_services(), ["test.New"]);
  assert!(registry.file_containing_symbol("test.Old").is_none());
  assert!(registry.file_containing_symbol("test.New").is_some());
  registry.map_symbol("alias", "api.proto");
  assert!(registry.file_containing_symbol("alias").is_some());
}

#[tokio::test]
async fn watching_unknown_services_does_not_register_them() {
  let registry = HealthRegistry::new();
  let mut watch = Box::pin(registry.watch("unknown"));
  assert_eq!(watch.next().await.unwrap().unwrap().status, 3);
  assert_eq!(registry.snapshot().len(), 0);
  registry.set_status("unknown", ServingStatus::Serving);
  assert_eq!(watch.next().await.unwrap().unwrap().status, 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn health_list_rejects_expired_deadlines() {
  let registry = HealthRegistry::new();
  registry.set_status("", ServingStatus::Serving);
  let mut router = Router::new();
  registry.register(&mut router);
  let request = http::Request::builder()
    .method("POST")
    .uri("/grpc.health.v1.Health/List")
    .header("content-type", "application/grpc")
    .header("grpc-timeout", "0n")
    .body(TakoBody::from(vec![0; 5]))
    .unwrap();

  let response = router.dispatch(request).await;

  assert_eq!(response.headers().get("grpc-status").unwrap(), "4");
}
