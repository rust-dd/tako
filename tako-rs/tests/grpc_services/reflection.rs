use std::net::SocketAddr;
use std::time::Duration;

use prost::Message;
use prost_types::FileDescriptorProto;
use tako::grpc::reflection::ReflectionRegistry;
use tonic_prost::ProstCodec;

use super::client;
use super::wire::*;

pub fn registry() -> ReflectionRegistry {
  let registry = ReflectionRegistry::new();
  registry
    .register_encoded_file_descriptor_set(include_bytes!("proto/services.bin"))
    .unwrap();
  registry.add_service("test.Math");
  registry.map_symbol("legacy.Alias", "service.proto");
  registry.add_file(
    "broken.proto",
    FileDescriptorProto {
      name: Some("broken.proto".into()),
      dependency: vec!["absent.proto".into()],
      ..Default::default()
    }
    .encode_to_vec(),
  );
  registry.add_file("corrupt.proto", vec![0xff]);
  registry
}

async fn stream(
  address: SocketAddr,
  version: &str,
) -> (
  tokio::sync::mpsc::Sender<ReflectionRequest>,
  tonic::Streaming<ReflectionResponse>,
) {
  let mut grpc = client(address).await;
  let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
  let outbound = futures_util::stream::poll_fn(move |cx| receiver.poll_recv(cx));
  grpc.ready().await.unwrap();
  let response = grpc
    .streaming(
      tonic::Request::new(outbound),
      format!("/grpc.reflection.{version}.ServerReflection/ServerReflectionInfo")
        .parse()
        .unwrap(),
      ProstCodec::<ReflectionRequest, ReflectionResponse>::new(),
    )
    .await
    .unwrap()
    .into_inner();
  (sender, response)
}

async fn query(
  sender: &tokio::sync::mpsc::Sender<ReflectionRequest>,
  replies: &mut tonic::Streaming<ReflectionResponse>,
  query: Option<Query>,
) -> Answer {
  let request = ReflectionRequest {
    host: "example.test".into(),
    query,
  };
  sender.send(request.clone()).await.unwrap();
  let reply = tokio::time::timeout(Duration::from_secs(2), replies.message())
    .await
    .expect("reflection waited for the request stream to close")
    .unwrap()
    .unwrap();
  assert_eq!(reply.valid_host, "example.test");
  assert_eq!(reply.original_request, Some(request));
  reply.answer.unwrap()
}

fn filenames(answer: Answer) -> Vec<String> {
  let Answer::Files(files) = answer else {
    panic!("expected file descriptors: {answer:?}")
  };
  let mut names = files
    .descriptors
    .iter()
    .map(|file| {
      FileDescriptorProto::decode(file.as_slice())
        .unwrap()
        .name
        .unwrap()
    })
    .collect::<Vec<_>>();
  names.sort();
  names
}

pub async fn lookups(address: SocketAddr) {
  let (sender, mut replies) = stream(address, "v1").await;
  let Answer::Services(services) =
    query(&sender, &mut replies, Some(Query::Services(String::new()))).await
  else {
    panic!("expected service list")
  };
  let names = services
    .services
    .into_iter()
    .map(|s| s.name)
    .collect::<Vec<_>>();
  assert_eq!(
    names,
    [
      "grpc.reflection.v1.ServerReflection",
      "grpc.reflection.v1alpha.ServerReflection",
      "test.Math"
    ]
  );
  for request in [
    Query::Filename("service.proto".into()),
    Query::Symbol("test.Math".into()),
    Query::Symbol("test.Math.Count".into()),
    Query::Symbol("test.Input.input".into()),
    Query::Symbol("test.note".into()),
    Query::Symbol("legacy.Alias".into()),
    Query::Extension(Extension {
      containing_type: "shared.Container".into(),
      number: 100,
    }),
  ] {
    assert_eq!(
      filenames(query(&sender, &mut replies, Some(request)).await),
      ["service.proto", "shared.proto"]
    );
  }
  for symbol in [
    "shared.Container.Nested",
    "shared.Container.Nested.value",
    "shared.Container.Mode",
    "shared.Container.ZERO",
    "shared.Container.selection",
  ] {
    assert_eq!(
      filenames(query(&sender, &mut replies, Some(Query::Symbol(symbol.into()))).await),
      ["shared.proto"]
    );
  }
  for (name, expected) in [
    ("shared.Container", vec![100]),
    ("shared.Container.Nested", vec![]),
  ] {
    let Answer::ExtensionNumbers(numbers) = query(
      &sender,
      &mut replies,
      Some(Query::ExtensionNumbers(name.into())),
    )
    .await
    else {
      panic!("expected extension numbers")
    };
    assert_eq!(numbers.base_type_name, name);
    assert_eq!(numbers.numbers, expected);
  }
  assert_eq!(
    filenames(
      query(
        &sender,
        &mut replies,
        Some(Query::Symbol("grpc.reflection.v1.ServerReflection".into()))
      )
      .await
    ),
    ["grpc/reflection/v1/reflection.proto"]
  );
  drop(sender);
  assert!(replies.message().await.unwrap().is_none());
}

pub async fn errors(address: SocketAddr) {
  let (sender, mut replies) = stream(address, "v1").await;
  for (request, code) in [
    (None, 3),
    (Some(Query::Filename("missing.proto".into())), 5),
    (Some(Query::Filename("broken.proto".into())), 5),
    (Some(Query::Filename("corrupt.proto".into())), 13),
    (Some(Query::Symbol("missing.Service".into())), 5),
    (
      Some(Query::Extension(Extension {
        containing_type: "shared.Container".into(),
        number: 101,
      })),
      5,
    ),
    (Some(Query::ExtensionNumbers("missing.Type".into())), 5),
  ] {
    let Answer::Error(error) = query(&sender, &mut replies, request).await else {
      panic!("expected error response")
    };
    assert_eq!(error.code, code);
    assert_ne!(error.message, "");
  }
  assert!(matches!(
    query(&sender, &mut replies, Some(Query::Services(String::new()))).await,
    Answer::Services(_)
  ));
  drop(sender);
  assert!(replies.message().await.unwrap().is_none());
}

pub async fn alpha(address: SocketAddr) {
  let (sender, mut replies) = stream(address, "v1alpha").await;
  assert_eq!(
    filenames(
      query(
        &sender,
        &mut replies,
        Some(Query::Symbol("test.Math".into()))
      )
      .await
    ),
    ["service.proto", "shared.proto"]
  );
  drop(sender);
  assert!(replies.message().await.unwrap().is_none());
}
