//! Regression test for the dual `simd-json` + `sonic-rs` backend story.
//!
//! Both crates are intentionally pulled in by the `simd` feature:
//! - `tako_core::extractors::json::Json` uses `sonic_rs` on its SIMD fast path.
//! - `tako_extractors::simdjson::SimdJson` uses `simd_json`.
//! - `tako_extractors::simdjson::SonicJson` uses `sonic_rs` directly.
//!
//! This test exercises all three so a future "pick one" refactor cannot
//! silently break either backend.

#![cfg(feature = "simd")]

use http::Method;
use serde::Deserialize;
use serde::Serialize;
use tako::body::TakoBody;
use tako::extractors::FromRequest;

#[derive(Debug, Deserialize, Serialize, PartialEq)]
struct Payload {
  name: String,
  age: u32,
  tags: Vec<String>,
}

fn sample_payload() -> &'static str {
  r#"{"name":"Alice","age":30,"tags":["a","b","c"]}"#
}

fn expected() -> Payload {
  Payload {
    name: "Alice".to_string(),
    age: 30,
    tags: vec!["a".to_string(), "b".to_string(), "c".to_string()],
  }
}

fn json_request(body: &'static str) -> http::Request<TakoBody> {
  http::Request::builder()
    .method(Method::POST)
    .uri("/api")
    .header("content-type", "application/json")
    .body(TakoBody::from(body))
    .unwrap()
}

#[tokio::test]
async fn json_simd_path_uses_sonic_rs() {
  use tako::extractors::json::Json;
  use tako::extractors::json::SimdJsonMode;

  // Force the SIMD branch so the route always goes through sonic_rs.
  let mut req = json_request(sample_payload());
  req.extensions_mut().insert(SimdJsonMode::Always);

  let Json(payload) = Json::<Payload>::from_request(&mut req).await.unwrap();
  assert_eq!(payload, expected());
}

#[tokio::test]
async fn json_simd_threshold_falls_back_to_serde_json() {
  use tako::extractors::json::Json;
  use tako::extractors::json::SimdJsonMode;

  // Threshold above the payload size — should fall back to serde_json.
  let mut req = json_request(sample_payload());
  req
    .extensions_mut()
    .insert(SimdJsonMode::Threshold(usize::MAX));

  let Json(payload) = Json::<Payload>::from_request(&mut req).await.unwrap();
  assert_eq!(payload, expected());
}

#[tokio::test]
async fn json_simd_never_disables_simd_branch() {
  use tako::extractors::json::Json;
  use tako::extractors::json::SimdJsonMode;

  let mut req = json_request(sample_payload());
  req.extensions_mut().insert(SimdJsonMode::Never);

  let Json(payload) = Json::<Payload>::from_request(&mut req).await.unwrap();
  assert_eq!(payload, expected());
}

#[tokio::test]
async fn simdjson_extractor_uses_simd_json_crate() {
  use tako::extractors::simdjson::SimdJson;

  let mut req = json_request(sample_payload());
  let SimdJson(payload) = SimdJson::<Payload>::from_request(&mut req).await.unwrap();
  assert_eq!(payload, expected());
}

#[tokio::test]
async fn sonicjson_extractor_uses_sonic_rs_crate() {
  use tako::extractors::simdjson::SonicJson;

  let mut req = json_request(sample_payload());
  let SonicJson(payload) = SonicJson::<Payload>::from_request(&mut req).await.unwrap();
  assert_eq!(payload, expected());
}
