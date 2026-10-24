use std::sync::atomic::AtomicBool;

use tako::middleware::IntoMiddleware;

use super::*;

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn explicit_route_order_checks_current_authorization_on_every_replay() {
  let authorized = Arc::new(AtomicBool::new(true));
  let auth_calls = Arc::new(AtomicUsize::new(0));
  let handler_calls = Arc::new(AtomicUsize::new(0));
  let allow_request = authorized.clone();
  let authenticate_calls = auth_calls.clone();
  let calls = handler_calls.clone();
  let mut router = Router::new();
  router
    .post("/charge", move || {
      calls.fetch_add(1, Ordering::SeqCst);
      async { "private receipt" }
    })
    .middleware(move |request, next| {
      authenticate_calls.fetch_add(1, Ordering::SeqCst);
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
    })
    .middleware(IdempotencyBuilder::new().build().into_middleware());

  assert_eq!(
    body(router.dispatch(post("alice")).await).await,
    "private receipt"
  );
  assert_eq!(
    body(router.dispatch(post("alice")).await).await,
    "private receipt"
  );
  assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
  assert_eq!(auth_calls.load(Ordering::SeqCst), 2);

  authorized.store(false, Ordering::SeqCst);
  assert_eq!(
    router.dispatch(post("alice")).await.status(),
    StatusCode::FORBIDDEN
  );
  assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
  assert_eq!(auth_calls.load(Ordering::SeqCst), 3);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn route_plugin_rejects_identity_supplied_only_by_later_route_authentication() {
  let calls = Arc::new(AtomicUsize::new(0));
  let handler_calls = calls.clone();
  let mut router = Router::new();
  router
    .post("/charge", move || {
      handler_calls.fetch_add(1, Ordering::SeqCst);
      async { "private receipt" }
    })
    .middleware(authenticate)
    .plugin(IdempotencyBuilder::new().build());
  assert_eq!(
    router.dispatch(post("alice")).await.status(),
    StatusCode::UNAUTHORIZED
  );
  assert_eq!(calls.load(Ordering::SeqCst), 0);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn nested_route_middleware_checks_child_authorization_before_replaying() {
  let authorized = Arc::new(AtomicBool::new(true));
  let authorization_calls = Arc::new(AtomicUsize::new(0));
  let handler_calls = Arc::new(AtomicUsize::new(0));
  let allow_request = authorized.clone();
  let authorize_calls = authorization_calls.clone();
  let calls = handler_calls.clone();
  let mut child = Router::new();
  child.middleware(move |request: Request, next: Next| {
    assert!(request.extensions().get::<IdempotencyIdentity>().is_some());
    authorize_calls.fetch_add(1, Ordering::SeqCst);
    let allowed = allow_request.load(Ordering::SeqCst);
    async move {
      if allowed {
        next.run(request).await
      } else {
        let mut response = Response::new(TakoBody::empty());
        *response.status_mut() = StatusCode::FORBIDDEN;
        response
      }
    }
  });
  child
    .post("/charge", move || {
      calls.fetch_add(1, Ordering::SeqCst);
      async { "private receipt" }
    })
    .middleware(IdempotencyBuilder::new().build().into_middleware());

  let mut root = Router::new();
  root.middleware(authenticate);
  root.nest("/api", child);
  let nested_post = || {
    let mut request = post("alice");
    *request.uri_mut() = "/api/charge".parse().unwrap();
    request
  };
  assert_eq!(
    body(root.dispatch(nested_post()).await).await,
    "private receipt"
  );
  assert_eq!(
    body(root.dispatch(nested_post()).await).await,
    "private receipt"
  );
  assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
  assert_eq!(authorization_calls.load(Ordering::SeqCst), 2);

  authorized.store(false, Ordering::SeqCst);
  assert_eq!(
    root.dispatch(nested_post()).await.status(),
    StatusCode::FORBIDDEN
  );
  assert_eq!(handler_calls.load(Ordering::SeqCst), 1);
  assert_eq!(authorization_calls.load(Ordering::SeqCst), 3);
}
