use http::Method;
use http::StatusCode;
use http::header;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;
use tako_rs_plugins::middleware::csrf::Csrf;
use tako_rs_plugins::middleware::session::Session;
use tako_rs_plugins::middleware::session::SessionMiddleware;
use tako_rs_plugins::stores::memory::MemoryCsrfTokenStore;

fn csrf(stored: bool) -> Csrf {
  if stored {
    Csrf::new().store(MemoryCsrfTokenStore::new())
  } else {
    Csrf::new()
  }
}

fn request(method: Method, path: &str, cookies: &str, token: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(path)
    .header(header::COOKIE, cookies)
    .header("x-csrf-token", token)
    .body(TakoBody::empty())
    .unwrap()
}

fn cookies(response: &Response) -> String {
  response
    .headers()
    .get_all(header::SET_COOKIE)
    .iter()
    .map(|value| value.to_str().unwrap().split(';').next().unwrap())
    .collect::<Vec<_>>()
    .join("; ")
}

fn token(response: &Response) -> &str {
  response
    .headers()
    .get_all(header::SET_COOKIE)
    .iter()
    .filter_map(|value| value.to_str().unwrap().strip_prefix("csrf_token="))
    .map(|value| value.split(';').next().unwrap())
    .next()
    .unwrap()
}

fn seeded_response() -> Response {
  http::Response::builder()
    .header(header::SET_COOKIE, "theme=dark; Path=/")
    .header(header::SET_COOKIE, "__csrf_seed=server-seed; Path=/")
    .header(header::SET_COOKIE, "__csrf_seed=another-seed; Path=/")
    .body(TakoBody::empty())
    .unwrap()
}

fn seed_router(stored: bool) -> Router {
  let mut router = Router::new();
  router.middleware(SessionMiddleware::new().into_middleware());
  router.middleware(csrf(stored).into_middleware());
  router.get("/", || async { "ready" });
  for method in [Method::GET, Method::POST] {
    router.route(method.clone(), "/seed", || async { seeded_response() });
    router.route(method, "/rotate", |request: Request| async move {
      request.extensions().get::<Session>().unwrap().rotate();
      seeded_response()
    });
  }
  router
}

fn assert_seed_removed(response: &Response) {
  assert_eq!(response.status(), StatusCode::OK);
  let cookies = cookies(response);
  assert!(!cookies.contains("__csrf_seed="), "{cookies}");
  assert!(cookies.contains("theme=dark"));
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn csrf_seed_is_removed_after_verified_unsafe_requests() {
  for stored in [false, true] {
    let router = seed_router(stored);
    let initial = router.dispatch(request(Method::GET, "/", "", "")).await;
    let response = router
      .dispatch(request(
        Method::POST,
        "/seed",
        &cookies(&initial),
        token(&initial),
      ))
      .await;

    assert_seed_removed(&response);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn csrf_seed_is_removed_when_session_rotation_ignores_it() {
  for stored in [false, true] {
    for method in [Method::GET, Method::POST] {
      let router = seed_router(stored);
      let initial = router.dispatch(request(Method::GET, "/", "", "")).await;
      let response = router
        .dispatch(request(
          method,
          "/rotate",
          &cookies(&initial),
          token(&initial),
        ))
        .await;

      assert_seed_removed(&response);
      assert_ne!(token(&response), "server-seed");
      assert_ne!(token(&response), token(&initial));
    }
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn safe_request_consumes_seed_and_preserves_unrelated_cookies() {
  let response = seed_router(false)
    .dispatch(request(Method::GET, "/seed", "", ""))
    .await;

  assert_seed_removed(&response);
  assert_eq!(token(&response), "server-seed");
}

async fn assert_exempt_boundaries(stored: bool) {
  for exemption in ["/webhook", "/webhook/"] {
    let mut router = Router::new();
    router.middleware(SessionMiddleware::new().into_middleware());
    router.middleware(csrf(stored).exempt(exemption).into_middleware());
    for path in [
      "/webhook",
      "/webhook/events",
      "/webhooks-admin",
      "/webhook-admin",
      "/other",
    ] {
      router.post(path, || async { StatusCode::ACCEPTED });
    }
    for path in ["/webhooks-admin", "/webhook-admin", "/other"] {
      let response = router.dispatch(request(Method::POST, path, "", "")).await;
      assert_eq!(
        response.status(),
        StatusCode::FORBIDDEN,
        "{exemption}: {path}"
      );
    }
    for path in ["/webhook", "/webhook/events"] {
      let response = router.dispatch(request(Method::POST, path, "", "")).await;
      assert_eq!(
        response.status(),
        StatusCode::ACCEPTED,
        "{exemption}: {path}"
      );
    }
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn session_bound_exemptions_only_match_complete_path_segments() {
  assert_exempt_boundaries(false).await;
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn stored_token_exemptions_only_match_complete_path_segments() {
  assert_exempt_boundaries(true).await;
}
