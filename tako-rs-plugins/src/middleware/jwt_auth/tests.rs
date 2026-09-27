use std::collections::HashMap;
use std::sync::Arc;

use jwt_simple::prelude::*;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::router::Router;
use tako_rs_core::types::BuildHasher;

use super::AnyVerifyKey;
use super::JwtAuth;
use super::MultiKeyVerifier;
use super::VerifyConstraints;
use crate::stores::VerificationKey;
use crate::stores::memory::StaticJwksProvider;

fn run(future: impl std::future::Future<Output = ()>) {
  #[cfg(not(feature = "compio"))]
  tokio::runtime::Builder::new_current_thread()
    .enable_all()
    .build()
    .unwrap()
    .block_on(future);
  #[cfg(feature = "compio")]
  compio::runtime::Runtime::new().unwrap().block_on(future);
}

fn verifier() -> MultiKeyVerifier<NoCustomClaims> {
  let mut keys = HashMap::with_hasher(BuildHasher::default());
  keys.insert(
    "HS256",
    AnyVerifyKey::HS256(Arc::new(HS256Key::from_bytes(&[1; 32]))),
  );
  MultiKeyVerifier::new(keys)
}

fn router(provider: StaticJwksProvider) -> Router {
  let mut router = Router::new();
  let auth = JwtAuth::new(verifier()).store(provider);
  router.middleware(auth.into_middleware());
  router.get("/", || async { "ok" });
  router
}

async fn status(router: &Router, token: &str) -> http::StatusCode {
  let req = http::Request::builder()
    .uri("/")
    .header("authorization", format!("Bearer {token}"))
    .body(TakoBody::empty())
    .unwrap();
  router.dispatch(req).await.status()
}

#[test]
fn provider_rotation_is_shared_and_algorithm_bound() {
  run(async {
    let provider = StaticJwksProvider::new();
    provider.insert(
      "current",
      VerificationKey {
        algorithm: "HS256".into(),
        bytes: vec![2; 32],
      },
    );
    let first = router(provider.clone());
    let second = router(provider.clone());
    let sign = |bytes: &[u8]| {
      HS256Key::from_bytes(bytes)
        .with_key_id("current")
        .authenticate(Claims::create(Duration::from_secs(60)))
        .unwrap()
    };
    let current = sign(&[2; 32]);
    assert_eq!(status(&first, &current).await, http::StatusCode::OK);
    assert_eq!(status(&second, &current).await, http::StatusCode::OK);
    let rotated = sign(&[3; 32]);
    assert_eq!(
      status(&first, &rotated).await,
      http::StatusCode::UNAUTHORIZED
    );
    provider.insert(
      "current",
      VerificationKey {
        algorithm: "HS256".into(),
        bytes: vec![3; 32],
      },
    );
    assert_eq!(status(&second, &rotated).await, http::StatusCode::OK);
    provider.insert(
      "asymmetric",
      VerificationKey {
        algorithm: "RS256".into(),
        bytes: vec![4; 32],
      },
    );
    let confusion = HS256Key::from_bytes(&[4; 32])
      .with_key_id("asymmetric")
      .authenticate(Claims::create(Duration::from_secs(60)))
      .unwrap();
    assert_eq!(
      status(&first, &confusion).await,
      http::StatusCode::UNAUTHORIZED
    );
    let static_token = sign(&[1; 32]);
    assert_eq!(
      status(&first, &static_token).await,
      http::StatusCode::UNAUTHORIZED
    );
    let fallback = HS256Key::from_bytes(&[1; 32])
      .with_key_id("unknown")
      .authenticate(Claims::create(Duration::from_secs(60)))
      .unwrap();
    assert_eq!(status(&first, &fallback).await, http::StatusCode::OK);
  });
}

#[test]
fn middleware_time_tolerance_reaches_the_verifier() {
  run(async {
    let key = HS256Key::from_bytes(&[1; 32]);
    let mut claims = Claims::create(Duration::from_secs(60)).with_issuer("expected");
    let now = Clock::now_since_epoch();
    claims.issued_at = Some(now - Duration::from_secs(60));
    claims.invalid_before = claims.issued_at;
    claims.expires_at = Some(now - Duration::from_secs(10));
    let token = key.authenticate(claims).unwrap();
    for (leeway, expected) in [
      (0, http::StatusCode::UNAUTHORIZED),
      (30, http::StatusCode::OK),
    ] {
      let mut router = Router::new();
      router.middleware(
        JwtAuth::new(verifier())
          .constraints(VerifyConstraints {
            issuer: Some("expected".into()),
            leeway_secs: leeway,
            ..Default::default()
          })
          .into_middleware(),
      );
      router.get("/", || async { "ok" });
      assert_eq!(status(&router, &token).await, expected);
    }
  });
}
