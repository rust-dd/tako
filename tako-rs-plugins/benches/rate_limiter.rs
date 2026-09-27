use std::hint::black_box;
use std::net::SocketAddr;

use criterion::Criterion;
use criterion::criterion_group;
use criterion::criterion_main;
use tako_rs_core::body::TakoBody;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_plugins::plugins::rate_limiter::RateLimiterBuilder;

fn rate_limit(c: &mut Criterion) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap();
  let router = runtime.block_on(async {
    let mut router = Router::new();
    router.get("/", || async { "ok" });
    router.plugin(
      RateLimiterBuilder::new()
        .max_requests(u32::MAX)
        .refill_rate(u32::MAX)
        .build(),
    );
    router.setup_plugins_once().unwrap();
    router
  });
  let peer = "192.0.2.1:1234".parse::<SocketAddr>().unwrap();
  c.bench_function("rate_limiter_peer_ip", |bencher| {
    bencher.iter(|| {
      runtime.block_on(async {
        let mut request = Request::new(TakoBody::empty());
        request.extensions_mut().insert(peer);
        black_box(router.dispatch(request).await)
      })
    });
  });
}
criterion_group!(benches, rate_limit);
criterion_main!(benches);
