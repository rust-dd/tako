#![allow(deprecated)]
use http::Method;
use http::StatusCode;
use http::Version;
use http_body_util::BodyExt;
use tako::body::TakoBody;
use tako::router::Router;
use tako::types::Request;

fn request(method: Method, uri: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::empty())
    .unwrap()
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn trailing_slash_redirect_preserves_the_query_verbatim() {
  for (route, uri, location) in [
    (
      "/users/",
      "/users?page=2&tag=a%2Fb&tag=c",
      "/users/?page=2&tag=a%2Fb&tag=c",
    ),
    ("/users", "/users/?page=2", "/users?page=2"),
    ("/users/", "/users?", "/users/?"),
  ] {
    let mut router = Router::new();
    router.route_with_tsr(Method::GET, route, || async { "users" });
    let response = router.dispatch(request(Method::GET, uri)).await;
    assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
    assert_eq!(response.headers()["location"], location);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn head_falls_back_to_get_and_keeps_representation_headers() {
  let mut router = Router::new();
  router.get("/users/{id}", |req: Request| async move {
    assert_eq!(req.method(), Method::HEAD);
    http::Response::builder()
      .header("content-type", "text/plain")
      .header("x-handler", "get")
      .body(TakoBody::from("hello"))
      .unwrap()
  });
  for version in [Version::HTTP_11, Version::HTTP_2, Version::HTTP_3] {
    let mut req = request(Method::HEAD, "/users/42");
    *req.version_mut() = version;
    let response = router.dispatch(req).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "text/plain");
    assert_eq!(response.headers()["content-length"], "5");
    assert_eq!(response.headers()["x-handler"], "get");
    assert!(
      response
        .into_body()
        .collect()
        .await
        .unwrap()
        .to_bytes()
        .is_empty()
    );
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn explicit_head_takes_precedence_and_its_final_body_is_removed() {
  let mut router = Router::new();
  router.get("/users", || async { "get" });
  router.route(Method::HEAD, "/users", || async {
    http::Response::builder()
      .status(StatusCode::ACCEPTED)
      .header("content-length", "42")
      .body(TakoBody::from("head"))
      .unwrap()
  });
  router.middleware(|req, next| async move {
    let mut response = next.run(req).await;
    *response.body_mut() = TakoBody::from("middleware body");
    response
  });
  let response = router.dispatch(request(Method::HEAD, "/users")).await;
  assert_eq!(response.status(), StatusCode::ACCEPTED);
  assert_eq!(response.headers()["content-length"], "42");
  assert!(
    response
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes()
      .is_empty()
  );
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn head_fallback_matches_per_path_and_redirects() {
  let mut router = Router::new();
  router.route(Method::HEAD, "/other", || async { "other" });
  router.route_with_tsr(Method::GET, "/users/", || async { "users" });
  let response = router.dispatch(request(Method::HEAD, "/users/?a=1")).await;
  assert_eq!(response.status(), StatusCode::OK);
  let response = router.dispatch(request(Method::HEAD, "/users?a=1")).await;
  assert_eq!(response.status(), StatusCode::TEMPORARY_REDIRECT);
  assert_eq!(response.headers()["location"], "/users/?a=1");
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn allow_includes_head_once_for_get_routes() {
  for explicit_head in [false, true] {
    let mut router = Router::new();
    router.get("/users", || async { "users" });
    if explicit_head {
      router.route(Method::HEAD, "/users", || async { "head" });
    }
    let response = router.dispatch(request(Method::POST, "/users")).await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    let mut methods = response.headers()["allow"]
      .to_str()
      .unwrap()
      .split(',')
      .map(str::trim)
      .collect::<Vec<_>>();
    methods.sort_unstable();
    assert_eq!(methods, ["GET", "HEAD"]);
  }
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn head_strips_fallback_and_error_formatter_bodies_without_polling() {
  let mut router = Router::new();
  router.fallback(|_| async { (StatusCode::NOT_FOUND, "missing") });
  router.client_error_handler(|mut response| {
    *response.body_mut() = TakoBody::from_stream(futures_util::stream::poll_fn::<
      Result<bytes::Bytes, std::io::Error>,
      _,
    >(|_| {
      panic!("HEAD must not poll the response stream")
    }));
    response
  });
  let response = router.dispatch(request(Method::HEAD, "/missing")).await;
  assert_eq!(response.status(), StatusCode::NOT_FOUND);
  assert!(!response.headers().contains_key("content-length"));
  assert!(
    response
      .into_body()
      .collect()
      .await
      .unwrap()
      .to_bytes()
      .is_empty()
  );
}

#[test]
fn typed_state_replaces_existing_values_without_affecting_other_routers() {
  #[derive(Clone)]
  struct Value(&'static str);

  let mut first = Router::new();
  let mut second = Router::new();
  first.with_state(Value("old"));
  let previous = first.router_state().get::<Value>().unwrap();
  first.with_state(Value("new"));
  second.with_state(Value("other"));
  assert_eq!(first.router_state().get::<Value>().unwrap().0, "new");
  assert_eq!(second.router_state().get::<Value>().unwrap().0, "other");
  assert_eq!(previous.0, "old");
  assert_eq!(first.router_state().len(), 1);

  tako::state::set_state(Value("old"));
  tako::state::set_state(Value("new"));
  assert_eq!(tako::state::get_state::<Value>().unwrap().0, "new");
}
