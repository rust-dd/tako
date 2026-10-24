use std::sync::atomic::AtomicBool;

use tako::stores::memory::MemoryIdempotencyStore;

use super::*;

fn public_router(builder: IdempotencyBuilder, calls: Arc<AtomicUsize>) -> Router {
  let mut router = Router::new();
  router.plugin(builder.build());
  router.post("/charge", move || {
    calls.fetch_add(1, Ordering::SeqCst);
    async { "public receipt" }
  });
  router
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn configured_extractor_scopes_existing_application_identity() {
  let calls = Arc::new(AtomicUsize::new(0));
  let mut router = Router::new();
  router.middleware(|request, next| async move {
    let mut request = request;
    let user = match request.headers()["authorization"].to_str().unwrap() {
      "alice" => "alice",
      "bob" => "bob",
      _ => panic!("unexpected test credential"),
    };
    request.extensions_mut().insert(Principal {
      user,
      tenant: "north",
    });
    next.run(request).await
  });
  router.plugin(
    IdempotencyBuilder::new()
      .identity_fn(|request| {
        request
          .extensions()
          .get::<Principal>()
          .map(|principal| IdempotencyIdentity::new(principal.user).tenant(principal.tenant))
      })
      .build(),
  );
  let handler_calls = calls.clone();
  router.post("/charge", move |request: Request| {
    handler_calls.fetch_add(1, Ordering::SeqCst);
    let user = request.extensions().get::<Principal>().unwrap().user;
    async move { user }
  });
  assert_eq!(body(router.dispatch(post("alice")).await).await, "alice");
  assert_eq!(body(router.dispatch(post("alice")).await).await, "alice");
  assert_eq!(body(router.dispatch(post("bob")).await).await, "bob");
  assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn missing_configured_identity_never_falls_back_to_the_extension() {
  let calls = Arc::new(AtomicUsize::new(0));
  let router = router_with(
    IdempotencyBuilder::new().identity_fn(|_| None),
    calls.clone(),
  );
  for _ in 0..2 {
    assert_eq!(
      router.dispatch(post("alice")).await.status(),
      StatusCode::UNAUTHORIZED
    );
  }
  assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn empty_principal_or_tenant_rejects_without_running_the_handler() {
  for identity in [
    IdempotencyIdentity::new(""),
    IdempotencyIdentity::new(" \t"),
    IdempotencyIdentity::new("alice").tenant(""),
    IdempotencyIdentity::new("alice").tenant(" \t"),
  ] {
    let calls = Arc::new(AtomicUsize::new(0));
    let router = public_router(
      IdempotencyBuilder::new().identity_fn(move |_| Some(identity.clone())),
      calls.clone(),
    );
    assert_eq!(
      router.dispatch(post("alice")).await.status(),
      StatusCode::UNAUTHORIZED
    );
    assert_eq!(calls.load(Ordering::SeqCst), 0);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn anonymous_scope_replays_only_after_explicit_opt_in() {
  let calls = Arc::new(AtomicUsize::new(0));
  let router = public_router(IdempotencyBuilder::new().anonymous(), calls.clone());
  assert_eq!(
    body(router.dispatch(post("alice")).await).await,
    "public receipt"
  );
  assert_eq!(
    body(router.dispatch(post("bob")).await).await,
    "public receipt"
  );
  assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn anonymous_entries_cannot_replay_authenticated_responses() {
  let store = MemoryIdempotencyStore::new();
  let calls = Arc::new(AtomicUsize::new(0));
  let authenticated = router_with(
    IdempotencyBuilder::new().store(store.clone()),
    calls.clone(),
  );
  let anonymous = public_router(
    IdempotencyBuilder::new().anonymous().store(store),
    calls.clone(),
  );
  assert_eq!(
    body(authenticated.dispatch(post("alice")).await).await,
    "private receipt for north/alice"
  );
  assert_eq!(
    body(anonymous.dispatch(post("alice")).await).await,
    "public receipt"
  );
  assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn route_authentication_after_global_idempotency_fails_closed() {
  let calls = Arc::new(AtomicUsize::new(0));
  let handler_calls = calls.clone();
  let mut router = Router::new();
  router.plugin(IdempotencyBuilder::new().build());
  router
    .post("/charge", move || {
      handler_calls.fetch_add(1, Ordering::SeqCst);
      async { "private receipt" }
    })
    .middleware(authenticate);
  assert_eq!(
    router.dispatch(post("alice")).await.status(),
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn revoked_authorization_is_checked_before_replaying_a_cached_response() {
  let authorized = Arc::new(AtomicBool::new(true));
  let calls = Arc::new(AtomicUsize::new(0));
  let handler_calls = calls.clone();
  let allow_request = authorized.clone();
  let mut router = Router::new();
  router.middleware(move |request, next| {
    let allowed = allow_request.load(Ordering::SeqCst);
    async move {
      if allowed {
        authenticate(request, next).await
      } else {
        let mut response = Response::new(TakoBody::empty());
        *response.status_mut() = StatusCode::FORBIDDEN;
        response
      }
    }
  });
  router.plugin(IdempotencyBuilder::new().build());
  router.post("/charge", move || {
    handler_calls.fetch_add(1, Ordering::SeqCst);
    async { "private receipt" }
  });
  assert_eq!(
    body(router.dispatch(post("alice")).await).await,
    "private receipt"
  );
  authorized.store(false, Ordering::SeqCst);
  assert_eq!(
    router.dispatch(post("alice")).await.status(),
    StatusCode::FORBIDDEN
  );
  assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn requests_without_an_idempotency_key_keep_the_existing_handler_behavior() {
  let calls = Arc::new(AtomicUsize::new(0));
  let router = public_router(IdempotencyBuilder::new(), calls.clone());
  let mut request = post("alice");
  request.headers_mut().remove("idempotency-key");
  assert_eq!(body(router.dispatch(request).await).await, "public receipt");
  assert_eq!(calls.load(Ordering::SeqCst), 1);
}
