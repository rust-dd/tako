#![cfg(feature = "hmac-signature")]

use std::time::Duration;
use std::time::SystemTime;
use std::time::UNIX_EPOCH;

use base64::Engine;
use hmac::Hmac;
use hmac::KeyInit;
use hmac::Mac;
use http::HeaderName;
use http::StatusCode;
use sha2::Sha256;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_plugins::middleware::hmac_signature::HmacSignature;

fn now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap()
    .as_secs()
}

fn signature(bytes: &[u8]) -> String {
  let mut mac = Hmac::<Sha256>::new_from_slice(b"secret").unwrap();
  mac.update(bytes);
  base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes())
}

fn verifier() -> HmacSignature {
  HmacSignature::new(HeaderName::from_static("x-signature"), b"secret".to_vec())
    .hex(false)
    .max_clock_skew(Duration::from_secs(300))
}

fn router(verifier: HmacSignature) -> Router {
  let mut router = Router::new();
  router.middleware(verifier.into_middleware());
  router.post("/hook", || async { StatusCode::ACCEPTED });
  router
}

fn request(timestamp: u64, signature: &str) -> Request {
  http::Request::builder()
    .method("POST")
    .uri("/hook")
    .header("x-event-time", timestamp.to_string())
    .header("x-signature", signature)
    .body(TakoBody::from("payload"))
    .unwrap()
}

#[tokio::test]
async fn custom_timestamp_rejects_replayed_signature_with_refreshed_header() {
  let router = router(verifier().timestamp_header(HeaderName::from_static("x-event-time")));
  let captured_signature = signature(b"POST /hook\npayload");

  let response = router.dispatch(request(now(), &captured_signature)).await;

  assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn configured_timestamp_is_signed_instead_of_conventional_header() {
  let router = router(verifier().timestamp_header(HeaderName::from_static("x-event-time")));
  let timestamp = now();
  let signed = signature(format!("POST /hook\n{timestamp}\npayload").as_bytes());
  let mut valid = request(timestamp, &signed);
  valid
    .headers_mut()
    .insert("x-timestamp", "1".parse().unwrap());

  assert_eq!(router.dispatch(valid).await.status(), StatusCode::ACCEPTED);
  assert_eq!(
    router
      .dispatch(request(timestamp + 1, &signed))
      .await
      .status(),
    StatusCode::UNAUTHORIZED
  );
}

#[tokio::test]
async fn custom_canonicalization_keeps_control_of_signed_bytes_in_either_builder_order() {
  fn canonical(parts: &http::request::Parts, body: &[u8]) -> Vec<u8> {
    let timestamp = parts.headers["x-event-time"].to_str().unwrap();
    let mut result = format!("{timestamp}.").into_bytes();
    result.extend_from_slice(body);
    result
  }

  for timestamp_first in [true, false] {
    let header = HeaderName::from_static("x-event-time");
    let verifier = if timestamp_first {
      verifier().timestamp_header(header).canonical(canonical)
    } else {
      verifier().canonical(canonical).timestamp_header(header)
    };
    let router = router(verifier);
    let timestamp = now();
    let signed = signature(format!("{timestamp}.payload").as_bytes());
    assert_eq!(
      router.dispatch(request(timestamp, &signed)).await.status(),
      StatusCode::ACCEPTED
    );
    assert_eq!(
      router
        .dispatch(request(timestamp + 1, &signed))
        .await
        .status(),
      StatusCode::UNAUTHORIZED
    );
  }
}

#[tokio::test]
async fn stale_signed_timestamp_is_rejected() {
  let router = router(verifier().timestamp_header(HeaderName::from_static("x-event-time")));
  let timestamp = now() - 600;
  let signed = signature(format!("POST /hook\n{timestamp}\npayload").as_bytes());

  assert_eq!(
    router.dispatch(request(timestamp, &signed)).await.status(),
    StatusCode::UNAUTHORIZED
  );
}

#[tokio::test]
async fn conventional_timestamp_still_signs_requests_without_configured_freshness() {
  let router = router(verifier());
  let signed = signature(b"POST /hook\n42\npayload");
  let mut request = request(now(), &signed);
  request
    .headers_mut()
    .insert("x-timestamp", "42".parse().unwrap());

  assert_eq!(
    router.dispatch(request).await.status(),
    StatusCode::ACCEPTED
  );
}
