//! Unary and streaming gRPC extraction from contiguous and fragmented DATA.

use std::convert::Infallible;
use std::hint::black_box;
use std::time::Duration;

use bytes::Bytes;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::Throughput;
use criterion::criterion_group;
use criterion::criterion_main;
use futures_util::StreamExt;
use http::Request;
use prost::Message;
use tako_rs_core::body::TakoBody;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::grpc::GrpcClientStream;
use tako_rs_core::grpc::GrpcRequest;
use tako_rs_core::grpc::grpc_encode;

#[derive(Clone, PartialEq, Message)]
struct Payload {
  #[prost(bytes = "bytes", tag = "1")]
  data: Bytes,
}

#[derive(Clone, PartialEq, Message)]
struct SumRequest {
  #[prost(int32, tag = "1")]
  a: i32,
  #[prost(int32, tag = "2")]
  b: i32,
}

fn request(body: TakoBody) -> Request<TakoBody> {
  Request::builder()
    .method("POST")
    .uri("/bench.Echo/Call")
    .header("content-type", "application/grpc")
    .body(body)
    .unwrap()
}

fn bench_decode(c: &mut Criterion) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap();
  let mut group = c.benchmark_group("grpc_decode");
  group.warm_up_time(Duration::from_secs(1));
  group.measurement_time(Duration::from_secs(3));
  group.sample_size(50);

  let sum = Bytes::from(grpc_encode(&SumRequest { a: 123, b: 456 }));
  group.throughput(Throughput::Elements(1));
  group.bench_function("unary_sum", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut request = request(TakoBody::from(sum.clone()));
        black_box(
          GrpcRequest::<SumRequest>::from_request(&mut request)
            .await
            .unwrap(),
        );
      });
    });
  });

  for size in [32, 1024, 64 * 1024] {
    let message = Payload {
      data: Bytes::from(vec![b'x'; size]),
    };
    let frame = Bytes::from(grpc_encode(&message));
    group.throughput(Throughput::Bytes(size as u64));
    group.bench_with_input(
      BenchmarkId::new("unary_contiguous", size),
      &frame,
      |b, frame| {
        b.iter(|| {
          runtime.block_on(async {
            let mut request = request(TakoBody::from(frame.clone()));
            black_box(
              GrpcRequest::<Payload>::from_request(&mut request)
                .await
                .unwrap(),
            );
          });
        });
      },
    );
    group.bench_with_input(
      BenchmarkId::new("unary_fragmented", size),
      &frame,
      |b, frame| {
        b.iter(|| {
          runtime.block_on(async {
            let chunks = [
              frame.slice(..3),
              frame.slice(3..frame.len() / 2),
              frame.slice(frame.len() / 2..),
            ];
            let body =
              TakoBody::from_stream(futures_util::stream::iter(chunks.map(Ok::<_, Infallible>)));
            let mut request = request(body);
            black_box(
              GrpcRequest::<Payload>::from_request(&mut request)
                .await
                .unwrap(),
            );
          });
        });
      },
    );
  }

  let message = Payload {
    data: Bytes::from(vec![b'x'; 32]),
  };
  let batch = Bytes::from(grpc_encode(&message).repeat(128));
  group.throughput(Throughput::Elements(128));
  group.bench_function("stream_128_batched", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut request = request(TakoBody::from(batch.clone()));
        let mut stream = GrpcClientStream::<Payload>::from_request(&mut request)
          .await
          .unwrap();
        while let Some(message) = stream.next().await {
          black_box(message.unwrap());
        }
      });
    });
  });
  group.finish();
}

criterion_group!(benches, bench_decode);
criterion_main!(benches);
