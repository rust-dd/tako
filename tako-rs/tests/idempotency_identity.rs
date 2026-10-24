#![cfg(feature = "plugins")]

use std::sync::Arc;
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::Ordering;

use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::middleware::Next;
use tako::plugins::idempotency::IdempotencyBuilder;
use tako::plugins::idempotency::IdempotencyIdentity;
use tako::plugins::idempotency::Scope;
use tako::router::Router;
use tako::types::Request;
use tako::types::Response;

#[path = "idempotency_identity/coalescing.rs"]
mod coalescing;
#[path = "idempotency_identity/configuration.rs"]
mod configuration;
#[path = "idempotency_identity/ordering.rs"]
mod ordering;

#[derive(Clone)]
struct Principal {
  user: &'static str,
  tenant: &'static str,
}

fn post(token: &str) -> Request {
  http::Request::builder()
    .method("POST")
    .uri("/charge")
    .header("authorization", token)
    .header("idempotency-key", "operation")
    .body(TakoBody::from("charge"))
    .unwrap()
}

async fn authenticate(mut request: Request, next: Next) -> Response {
  let (user, tenant) = match request.headers()["authorization"].to_str().unwrap() {
    "alice" | "alice-refreshed" => ("alice", "north"),
    "bob" => ("bob", "north"),
    "alice-south" => ("alice", "south"),
    "delimited-tenant" => ("bob", "north:alice"),
    "delimited-user" => ("alice:bob", "north"),
    _ => {
      let mut response = Response::new(TakoBody::empty());
      *response.status_mut() = StatusCode::UNAUTHORIZED;
      return response;
    }
  };
  request.extensions_mut().insert(Principal { user, tenant });
  request
    .extensions_mut()
    .insert(IdempotencyIdentity::new(user).tenant(tenant));
  next.run(request).await
}

fn router(calls: Arc<AtomicUsize>) -> Router {
  router_with(IdempotencyBuilder::new(), calls)
}

fn router_with(builder: IdempotencyBuilder, calls: Arc<AtomicUsize>) -> Router {
  let mut router = Router::new();
  router.middleware(authenticate);
  router.plugin(builder.build());
  router.post("/charge", move |request: Request| {
    calls.fetch_add(1, Ordering::SeqCst);
    let principal = request.extensions().get::<Principal>().unwrap().clone();
    async move {
      format!(
        "private receipt for {}/{}",
        principal.tenant, principal.user
      )
    }
  });
  router
}

async fn body(response: Response) -> String {
  assert_eq!(response.status(), StatusCode::OK);
  String::from_utf8(
    response
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes()
      .to_vec(),
  )
  .unwrap()
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn cached_responses_are_isolated_by_authenticated_principal() {
  for scope in [Scope::KeyOnly, Scope::MethodAndPath] {
    let calls = Arc::new(AtomicUsize::new(0));
    let router = router_with(IdempotencyBuilder::new().scope(scope), calls.clone());
    assert_eq!(
      body(router.dispatch(post("alice")).await).await,
      "private receipt for north/alice"
    );
    assert_eq!(
      body(router.dispatch(post("bob")).await).await,
      "private receipt for north/bob"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn cached_responses_are_isolated_by_authenticated_tenant() {
  for scope in [Scope::KeyOnly, Scope::MethodAndPath] {
    let calls = Arc::new(AtomicUsize::new(0));
    let router = router_with(IdempotencyBuilder::new().scope(scope), calls.clone());
    assert_eq!(
      body(router.dispatch(post("alice")).await).await,
      "private receipt for north/alice"
    );
    assert_eq!(
      body(router.dispatch(post("alice-south")).await).await,
      "private receipt for south/alice"
    );
    assert_eq!(calls.load(Ordering::SeqCst), 2);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn same_principal_retry_replays_after_credentials_refresh() {
  let calls = Arc::new(AtomicUsize::new(0));
  let router = router(calls.clone());
  assert_eq!(
    body(router.dispatch(post("alice")).await).await,
    "private receipt for north/alice"
  );
  assert_eq!(
    body(router.dispatch(post("alice-refreshed")).await).await,
    "private receipt for north/alice"
  );
  assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn identity_components_cannot_collide_through_delimiters() {
  let calls = Arc::new(AtomicUsize::new(0));
  let router = router(calls.clone());
  assert_eq!(
    body(router.dispatch(post("delimited-tenant")).await).await,
    "private receipt for north:alice/bob"
  );
  assert_eq!(
    body(router.dispatch(post("delimited-user")).await).await,
    "private receipt for north/alice:bob"
  );
  assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn missing_trusted_identity_rejects_before_running_the_handler() {
  let calls = Arc::new(AtomicUsize::new(0));
  let handler_calls = calls.clone();
  let mut router = Router::new();
  router.plugin(IdempotencyBuilder::new().build());
  router.post("/charge", move || {
    handler_calls.fetch_add(1, Ordering::SeqCst);
    async { "private receipt" }
  });
  for _ in 0..2 {
    assert_eq!(
      router.dispatch(post("alice")).await.status(),
      StatusCode::UNAUTHORIZED
    );
  }
  assert_eq!(calls.load(Ordering::SeqCst), 0);
}
