use std::hint::black_box;
use std::time::Instant;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_plugins::plugins::compression::CompressionBuilder;

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
      })
    });
  });
  health_latency.sort_unstable();
  println!(
    "health request latency: median {:?}, p95 {:?}, samples {}",
    health_latency[health_latency.len() / 2],
    health_latency[health_latency.len() * 95 / 100],
    health_latency.len()
  );
}

criterion_group!(benches, mixed_requests);
criterion_main!(benches);
