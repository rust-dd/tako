use std::time::Duration;

use async_trait::async_trait;
use base64::Engine as _;
use tako::middleware::csrf::Csrf;
use tako::middleware::jwt_auth::JwtAuth;
use tako::middleware::jwt_auth::JwtVerifier;
use tako::middleware::session::SessionMiddleware;
use tako::stores::*;

use super::*;

#[derive(Clone)]
struct Offline;
fn offline<T>() -> StoreResult<T> {
  Err(std::io::Error::other("private backend credentials").into())
}
#[async_trait]
impl SessionStore for Offline {
  async fn load(&self, _: &str) -> StoreResult<Option<Vec<u8>>> {
    offline()
  }
  async fn store(&self, _: &str, _: Vec<u8>, _: Duration) -> StoreResult<()> {
    offline()
  }
  async fn remove(&self, _: &str) -> StoreResult<bool> {
    offline()
  }
}
#[async_trait]
impl CsrfTokenStore for Offline {
  async fn issue(&self, _: &str, _: Duration) -> StoreResult<String> {
    offline()
  }
  async fn validate(&self, _: &str, _: &str, _: bool) -> StoreResult<bool> {
    offline()
  }
}
#[async_trait]
impl RateLimitStore for Offline {
  async fn consume(&self, _: &str, _: u32) -> StoreResult<RateLimitDecision> {
    offline()
  }
}
#[async_trait]
impl IdempotencyStore for Offline {
  async fn get(&self, _: &str) -> StoreResult<Option<IdempotencyEntry>> {
    offline()
  }
  async fn begin(&self, _: &str, _: [u8; 32]) -> StoreResult<IdempotencyBegin> {
    offline()
  }
  async fn complete(
    &self,
    _: &str,
    _: &str,
    _: IdempotencyEntry,
    _: Duration,
  ) -> StoreResult<bool> {
    offline()
  }
  async fn remove(&self, _: &str, _: &str) -> StoreResult<bool> {
    offline()
  }
}
#[async_trait]
impl JwksProvider for Offline {
  async fn keys_for(&self, _: &str) -> StoreResult<Vec<VerificationKey>> {
    offline()
  }
}
impl JwtVerifier for Offline {
  type Claims = ();
  type Error = &'static str;
  fn verify(&self, _: &str) -> Result<(), Self::Error> {
    Ok(())
  }
}
async fn unavailable(router: Router, request: Request) {
  let response = router.dispatch(request).await;
  assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
  assert!(!body(response).await.contains("credentials"));
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn middleware_backend_failures_are_503_and_do_not_leak_details() {
  let mut session = Router::new();
  session.middleware(SessionMiddleware::new().store(Offline).into_middleware());
  session.get("/", must_not_run);
  unavailable(session, request(Method::GET, "/", "tako_session=existing")).await;

  let mut csrf = Router::new();
  csrf.middleware(SessionMiddleware::new().into_middleware());
  csrf.middleware(Csrf::new().store(Offline).into_middleware());
  csrf.get("/", || async { "ok" });
  unavailable(csrf, request(Method::GET, "/", "")).await;

  let mut jwt = Router::new();
  jwt.middleware(JwtAuth::new(Offline).store(Offline).into_middleware());
  jwt.get("/", must_not_run);
  let token = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(br#"{"kid":"rotated"}"#);
  let mut req = request(Method::GET, "/", "");
  req.headers_mut().insert(
    "authorization",
    format!("Bearer {token}.claims.signature").parse().unwrap(),
  );
  unavailable(jwt, req).await;
}

#[cfg(feature = "plugins")]
#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn plugin_backend_failures_stop_dispatch() {
  let mut rate = Router::new();
  rate.plugin(
    tako::plugins::rate_limiter::RateLimiterBuilder::new()
      .store(Offline)
      .key_fn(|_| Some("user".into()))
      .build(),
  );
  rate.get("/", must_not_run);
  unavailable(rate, request(Method::GET, "/", "")).await;
  let mut idempotency = Router::new();
  idempotency.plugin(
    tako::plugins::idempotency::IdempotencyBuilder::new()
      .store(Offline)
      .build(),
  );
  idempotency.post("/", must_not_run);
  let mut req = request(Method::POST, "/", "");
  req
    .headers_mut()
    .insert("idempotency-key", "key".parse().unwrap());
  unavailable(idempotency, req).await;
}

async fn must_not_run() -> &'static str {
  panic!("failed backend must stop dispatch");
}
