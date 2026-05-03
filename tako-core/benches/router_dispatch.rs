//! Hot-path bench: router dispatch for static and dynamic paths.
//!
//! Run with: `cargo bench -p tako-core --bench router_dispatch`.

use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use http::Method;
use http::Request;
use tako_core::body::TakoBody;
use tako_core::responder::Responder;
use tako_core::router::Router;

async fn ok() -> impl Responder {
  "ok"
}

fn build_router() -> Router {
  let mut r = Router::new();
  r.get("/health", ok);
  r.get("/users", ok);
  r.get("/users/{id}", ok);
  r.get("/orgs/{org}/projects/{project}", ok);
  r.post("/users", ok);
  r.delete("/users/{id}", ok);
  r
}

fn bench_dispatch(c: &mut Criterion) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap();
  let router = build_router();

  let mut group = c.benchmark_group("router_dispatch");

  group.bench_function("static_path", |b| {
    b.iter(|| {
      let req = Request::builder()
        .method(Method::GET)
        .uri("/health")
        .body(TakoBody::empty())
        .unwrap();
      runtime.block_on(async { black_box(router.dispatch(req).await) })
    });
  });

  group.bench_function("dynamic_one_param", |b| {
    b.iter(|| {
      let req = Request::builder()
        .method(Method::GET)
        .uri("/users/42")
        .body(TakoBody::empty())
        .unwrap();
      runtime.block_on(async { black_box(router.dispatch(req).await) })
    });
  });

  group.bench_function("dynamic_two_params", |b| {
    b.iter(|| {
      let req = Request::builder()
        .method(Method::GET)
        .uri("/orgs/acme/projects/widget")
        .body(TakoBody::empty())
        .unwrap();
      runtime.block_on(async { black_box(router.dispatch(req).await) })
    });
  });

  group.bench_function("method_mismatch_405", |b| {
    b.iter(|| {
      let req = Request::builder()
        .method(Method::PUT)
        .uri("/users")
        .body(TakoBody::empty())
        .unwrap();
      runtime.block_on(async { black_box(router.dispatch(req).await) })
    });
  });

  group.finish();
}

criterion_group!(benches, bench_dispatch);
criterion_main!(benches);
