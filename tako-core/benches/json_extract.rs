//! Hot-path bench: `Json<T>` extraction across the simd/serde split.
//!
//! Run with: `cargo bench -p tako-core --bench json_extract`.

use criterion::Criterion;
use criterion::black_box;
use criterion::criterion_group;
use criterion::criterion_main;
use http::Method;
use http::Request;
use serde::Deserialize;
use serde::Serialize;
use tako_core::body::TakoBody;
use tako_core::extractors::FromRequest;
use tako_core::extractors::json::Json;
#[cfg(feature = "simd")]
use tako_core::extractors::json::SimdJsonMode;

#[derive(Deserialize, Serialize)]
struct Payload {
  name: String,
  age: u32,
  tags: Vec<String>,
}

const SMALL_BODY: &str = r#"{"name":"alice","age":30,"tags":["a","b","c"]}"#;
const LARGE_BODY_SIZE: usize = 64 * 1024;

fn make_large_body() -> String {
  let mut tags = String::with_capacity(LARGE_BODY_SIZE);
  tags.push('[');
  for i in 0..512 {
    if i > 0 {
      tags.push(',');
    }
    tags.push_str(&format!("\"tag{i:04}\""));
  }
  tags.push(']');
  format!(r#"{{"name":"alice","age":30,"tags":{tags}}}"#)
}

fn json_request(body: String) -> Request<TakoBody> {
  Request::builder()
    .method(Method::POST)
    .uri("/api")
    .header("content-type", "application/json")
    .body(TakoBody::from(body))
    .unwrap()
}

fn bench_json(c: &mut Criterion) {
  let runtime = tokio::runtime::Builder::new_current_thread()
    .build()
    .unwrap();

  let mut group = c.benchmark_group("json_extract");
  let large = make_large_body();

  group.bench_function("small_default", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut req = json_request(SMALL_BODY.to_string());
        let _ = black_box(Json::<Payload>::from_request(&mut req).await);
      });
    });
  });

  #[cfg(feature = "simd")]
  group.bench_function("small_force_simd", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut req = json_request(SMALL_BODY.to_string());
        req.extensions_mut().insert(SimdJsonMode::Always);
        let _ = black_box(Json::<Payload>::from_request(&mut req).await);
      });
    });
  });

  group.bench_function("large_default", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut req = json_request(large.clone());
        let _ = black_box(Json::<Payload>::from_request(&mut req).await);
      });
    });
  });

  #[cfg(feature = "simd")]
  group.bench_function("large_force_simd", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut req = json_request(large.clone());
        req.extensions_mut().insert(SimdJsonMode::Always);
        let _ = black_box(Json::<Payload>::from_request(&mut req).await);
      });
    });
  });

  #[cfg(feature = "simd")]
  group.bench_function("large_force_serde", |b| {
    b.iter(|| {
      runtime.block_on(async {
        let mut req = json_request(large.clone());
        req.extensions_mut().insert(SimdJsonMode::Never);
        let _ = black_box(Json::<Payload>::from_request(&mut req).await);
      });
    });
  });

  group.finish();
}

criterion_group!(benches, bench_json);
criterion_main!(benches);
