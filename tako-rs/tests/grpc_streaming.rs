#![cfg(all(feature = "grpc", feature = "http2"))]

//! Drives every RPC shape through a tonic client over h2c.

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;
use std::time::Duration;

use futures_util::StreamExt;
use http::uri::PathAndQuery;
use prost::Message;
use tako::Method;
use tako::grpc::GrpcBidi;
use tako::grpc::GrpcClientStream;
use tako::grpc::GrpcDeadline;
use tako::grpc::GrpcRequest;
use tako::grpc::GrpcResponse;
use tako::grpc::GrpcServerStream;
use tako::grpc::GrpcStatus;
use tako::grpc::GrpcStatusCode;
use tako::grpc::MAX_GRPC_MESSAGE_SIZE;
use tako::responder::Responder;
use tako::router::Router;
use tonic::Code;
use tonic::transport::Channel;
use tonic_prost::ProstCodec;

#[derive(Clone, PartialEq, Message)]
struct Number {
  #[prost(int64, tag = "1")]
  value: i64,
}

#[derive(Clone, PartialEq, Message)]
struct Blob {
  #[prost(bytes = "vec", tag = "1")]
  data: Vec<u8>,
}

fn number(value: i64) -> Number {
  Number { value }
}

/// Counts drops of the stream a cancelled call leaves behind.
struct DropCounter(Arc<AtomicUsize>);

impl Drop for DropCounter {
  fn drop(&mut self) {
    self.0.fetch_add(1, Ordering::SeqCst);
  }
}

async fn double(req: GrpcRequest<Number>) -> GrpcResponse<Number> {
  GrpcResponse::ok(number(req.message.value * 2))
}

async fn size(req: GrpcRequest<Blob>) -> GrpcResponse<Number> {
  GrpcResponse::ok(number(req.message.data.len() as i64))
}

async fn count(req: GrpcRequest<Number>) -> impl Responder {
  GrpcServerStream::new(futures_util::stream::iter(1..=req.message.value).map(|v| Ok(number(v))))
}

async fn sum(mut numbers: GrpcClientStream<Number>) -> Result<GrpcResponse<Number>, GrpcStatus> {
  let mut total = 0;
  while let Some(n) = numbers.next().await {
    total += n?.value;
  }
  Ok(GrpcResponse::ok(number(total)))
}

async fn scale(bidi: GrpcBidi<Number, Number>) -> impl Responder {
  bidi.respond(|numbers| numbers.map(|n| n.map(|n| number(n.value * 10)).map_err(GrpcStatus::from)))
}

async fn fail() -> impl Responder {
  GrpcServerStream::new(futures_util::stream::iter([
    Ok(number(1)),
    Err(GrpcStatus::error(GrpcStatusCode::InvalidArgument, "boom")),
    Ok(number(2)),
  ]))
}

async fn stall(deadline: Option<GrpcDeadline>, _req: GrpcRequest<Number>) -> impl Responder {
  let first = futures_util::stream::once(async { Ok(number(1)) });
  GrpcServerStream::new(first.chain(futures_util::stream::pending())).with_deadline(deadline)
}

fn router(dropped: &Arc<AtomicUsize>) -> Router {
  let mut router = Router::new();
  router.route(Method::POST, "/test.Math/Double", double);
  router.route(Method::POST, "/test.Math/Size", size);
  router.route(Method::POST, "/test.Math/Count", count);
  router.route(Method::POST, "/test.Math/Sum", sum);
  router.route(Method::POST, "/test.Math/Scale", scale);
  router.route(Method::POST, "/test.Math/Fail", fail);
  router.route(Method::POST, "/test.Math/Stall", stall);
  let dropped = dropped.clone();
  router.route(
    Method::POST,
    "/test.Math/Endless",
    move |_: GrpcRequest<Number>| {
      let counter = DropCounter(dropped.clone());
      async move {
        GrpcServerStream::new(futures_util::stream::repeat_with(move || {
          let _ = &counter;
          Ok(number(1))
        }))
      }
    },
  );
  router
}

fn path(method: &str) -> PathAndQuery {
  format!("/test.Math/{method}").parse().unwrap()
}

async fn client(address: SocketAddr) -> tonic::client::Grpc<Channel> {
  let channel = tonic::transport::Endpoint::from_shared(format!("http://{address}"))
    .unwrap()
    .connect()
    .await
    .unwrap();
  tonic::client::Grpc::new(channel)
}

async fn exercise(address: SocketAddr, dropped: Arc<AtomicUsize>) {
  let mut grpc = client(address).await;
  let codec = ProstCodec::<Number, Number>::new;

  grpc.ready().await.unwrap();
  let reply = grpc
    .unary(tonic::Request::new(number(21)), path("Double"), codec())
    .await
    .unwrap();
  assert_eq!(reply.into_inner().value, 42);

  grpc.ready().await.unwrap();
  let mut counted = grpc
    .server_streaming(tonic::Request::new(number(3)), path("Count"), codec())
    .await
    .unwrap()
    .into_inner();
  let mut values = Vec::new();
  while let Some(n) = counted.message().await.unwrap() {
    values.push(n.value);
  }
  assert_eq!(values, [1, 2, 3]);

  grpc.ready().await.unwrap();
  let numbers = futures_util::stream::iter([1, 2, 3, 4].map(number));
  let total = grpc
    .client_streaming(tonic::Request::new(numbers), path("Sum"), codec())
    .await
    .unwrap();
  assert_eq!(total.into_inner().value, 10);

  // Each reply arrives before the next request is sent, so both directions
  // are open at once.
  let (sender, mut receiver) = tokio::sync::mpsc::channel(1);
  let outbound = futures_util::stream::poll_fn(move |cx| receiver.poll_recv(cx));
  grpc.ready().await.unwrap();
  let mut replies = grpc
    .streaming(tonic::Request::new(outbound), path("Scale"), codec())
    .await
    .unwrap()
    .into_inner();
  for value in [1, 2, 3] {
    sender.send(number(value)).await.unwrap();
    assert_eq!(replies.message().await.unwrap().unwrap().value, value * 10);
  }
  drop(sender);
  assert!(replies.message().await.unwrap().is_none());

  grpc.ready().await.unwrap();
  let mut failing = grpc
    .server_streaming(tonic::Request::new(number(0)), path("Fail"), codec())
    .await
    .unwrap()
    .into_inner();
  assert_eq!(failing.message().await.unwrap().unwrap().value, 1);
  let status = failing.message().await.unwrap_err();
  assert_eq!(
    (status.code(), status.message()),
    (Code::InvalidArgument, "boom")
  );

  // tonic only sends `grpc-timeout`; the server has to end the call.
  let mut request = tonic::Request::new(number(0));
  request.set_timeout(Duration::from_millis(100));
  grpc.ready().await.unwrap();
  let mut stalled = grpc
    .server_streaming(request, path("Stall"), codec())
    .await
    .unwrap()
    .into_inner();
  assert_eq!(stalled.message().await.unwrap().unwrap().value, 1);
  let status = stalled.message().await.unwrap_err();
  assert_eq!(status.code(), Code::DeadlineExceeded);

  grpc.ready().await.unwrap();
  let mut endless = grpc
    .server_streaming(tonic::Request::new(number(0)), path("Endless"), codec())
    .await
    .unwrap()
    .into_inner();
  assert_eq!(endless.message().await.unwrap().unwrap().value, 1);
  drop(endless);
  tokio::time::timeout(Duration::from_secs(5), async {
    while dropped.load(Ordering::SeqCst) == 0 {
      tokio::time::sleep(Duration::from_millis(10)).await;
    }
  })
  .await
  .expect("the server kept a cancelled stream alive");

  grpc.ready().await.unwrap();
  let blob = Blob {
    data: vec![0; MAX_GRPC_MESSAGE_SIZE + 1],
  };
  let status = grpc
    .unary(
      tonic::Request::new(blob),
      path("Size"),
      ProstCodec::<Blob, Number>::new(),
    )
    .await
    .unwrap_err();
  assert_eq!(status.code(), Code::ResourceExhausted);
}

#[cfg(not(feature = "compio"))]
#[tokio::test]
async fn tonic_client_calls_every_rpc_shape_on_tokio() {
  let dropped = Arc::new(AtomicUsize::new(0));
  let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
  let address = listener.local_addr().unwrap();
  let handle = tako::Server::builder()
    .build()
    .try_spawn_h2c(listener, router(&dropped))
    .unwrap();
  tokio::time::timeout(Duration::from_secs(20), exercise(address, dropped))
    .await
    .expect("gRPC calls timed out");
  handle.shutdown(Duration::from_secs(1)).await;
}

#[cfg(feature = "compio")]
#[test]
fn tonic_client_calls_every_rpc_shape_on_compio() {
  compio::runtime::Runtime::new().unwrap().block_on(async {
    let dropped = Arc::new(AtomicUsize::new(0));
    let listener = compio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let handle = tako::CompioServer::builder()
      .build()
      .try_spawn_h2c(listener, router(&dropped))
      .unwrap();
    let (done, finished) = tokio::sync::oneshot::channel();
    let client = std::thread::spawn(move || {
      let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
      runtime.block_on(exercise(address, dropped));
      let _ = done.send(());
    });
    if compio::time::timeout(Duration::from_secs(20), finished)
      .await
      .is_err()
    {
      panic!("gRPC calls timed out");
    }
    if let Err(panic) = client.join() {
      std::panic::resume_unwind(panic);
    }
    handle.shutdown(Duration::from_secs(1)).await;
  });
}
