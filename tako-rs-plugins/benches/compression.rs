use std::hint::black_box;
use std::time::Instant;

use bytes::Bytes;
use criterion::BatchSize;
use criterion::BenchmarkId;
use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use futures_util::FutureExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_plugins::plugins::compression::CompressionBuilder;

fn buffered_responses(c: &mut Criterion) {
  let mut group = c.benchmark_group("compression_buffered");
  for size in [128, 1024, 16 * 1024] {
    let payload = Bytes::from(format!(
      r#"{{"message":"{}"}}"#,
      "response ".repeat(size / 9)
    ));
    let mut router = Router::new();
    router.plugin(CompressionBuilder::new().build());
    router.get("/", move || {
      let payload = payload.clone();
      async move {
        http::Response::builder()
          .header("content-type", "application/json")
          .body(TakoBody::from(payload))
          .unwrap()
      }
    });
    for encoding in ["identity", "gzip", "br", "deflate"] {
      group.bench_with_input(
        BenchmarkId::new(encoding, size),
        &encoding,
        |b, encoding| {
          b.iter_batched(
            || {
              http::Request::builder()
                .header("accept-encoding", *encoding)
                .body(TakoBody::empty())
                .unwrap()
            },
            |request| black_box(router.dispatch(request).now_or_never().unwrap()),
            BatchSize::SmallInput,
          );
        },
      );
    }
  }
  group.finish();
}

fn negotiation(c: &mut Criterion) {
  let mut router = Router::new();
  router.plugin(CompressionBuilder::new().build());
  router.get("/", || async { http::StatusCode::NO_CONTENT });
  let mut group = c.benchmark_group("compression_negotiation");
  for (label, accepted) in [
    ("empty", String::new()),
    ("gzip", "gzip".into()),
    ("browser", "gzip, deflate, br, zstd".into()),
    (
      "parameters",
      "gzip;q=0.5, br;q=0, deflate;q=1, *;q=0".into(),
    ),
    ("unknown_tokens", "unknown, ".repeat(128)),
  ] {
    group.bench_function(label, |b| {
      b.iter_batched(
        || {
          http::Request::builder()
            .header("accept-encoding", accepted.as_str())
            .body(TakoBody::empty())
            .unwrap()
        },
        |request| black_box(router.dispatch(request).now_or_never().unwrap()),
        BatchSize::SmallInput,
      );
    });
  }
  group.finish();
}

fn mixed_requests(c: &mut Criterion) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
  let mut seed = 42u64;
  let body = (0..512 * 1024)
    .map(|_| {
      seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
      char::from(32 + u8::try_from((seed >> 32) % 95).unwrap())
    })
    .collect::<String>();
  let mut router = Router::new();
  router.plugin(CompressionBuilder::new().build());
  router.get("/large", move || {
    let body = body.clone();
    async move { body }
  });
  router.get("/health", || async { "ok" });
  let mut health_latency = Vec::new();
  c.bench_function("compression_mixed_requests", |bencher| {
    bencher.iter(|| {
      runtime.block_on(async {
        let started = Instant::now();
        let large = http::Request::builder()
          .uri("/large")
          .header("accept-encoding", "gzip")
          .body(TakoBody::empty())
          .unwrap();
        let compress = router.dispatch(large);
        let health = async {
          let request = http::Request::builder()
            .uri("/health")
            .body(TakoBody::empty())
            .unwrap();
          black_box(router.dispatch(request).await);
          health_latency.push(started.elapsed());
        };
        let (response, ()) = futures_util::join!(compress, health);
        black_box(response);
      });
    });
  });
  if health_latency.is_empty() {
    return;
  }
  health_latency.sort_unstable();
  println!(
    "health request latency: median {:?}, p95 {:?}, samples {}",
    health_latency[health_latency.len() / 2],
    health_latency[health_latency.len() * 95 / 100],
    health_latency.len()
  );
}

criterion_group!(benches, buffered_responses, negotiation, mixed_requests);
criterion_main!(benches);
