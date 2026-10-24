use std::collections::BTreeMap;
use std::sync::Arc;

use http::Method;
use http::StatusCode;
use tako::body::TakoBody;
use tako::middleware::IntoMiddleware;
use tako::middleware::csrf::Csrf;
use tako::middleware::session::Session;
use tako::middleware::session::SessionMiddleware;
use tako::router::Router;
use tako::stores::memory::MemoryCsrfTokenStore;
use tako::types::Request;
use tako::types::Response;
use tokio::sync::Notify;

fn apply_cookies(jar: &mut BTreeMap<String, String>, response: &Response) {
  for cookie in response.headers().get_all("set-cookie") {
    let (key, value) = cookie
      .to_str()
      .unwrap()
      .split(';')
      .next()
      .unwrap()
      .split_once('=')
      .unwrap();
    jar.insert(key.into(), value.into());
  }
}

fn request(method: Method, path: &str, jar: &BTreeMap<String, String>) -> Request {
  http::Request::builder()
    .method(method)
    .uri(path)
    .header(
      "cookie",
      jar
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join("; "),
    )
    .header(
      "x-csrf-token",
      jar.get("app_csrf").map_or("", String::as_str),
    )
    .body(TakoBody::empty())
    .unwrap()
}

async fn late_response_preserves_rotated_csrf(stored: bool) {
  let mut router = Router::new();
  let entered = Arc::new(Notify::new());
  let release = Arc::new(Notify::new());
  router.middleware(SessionMiddleware::new().into_middleware());
  let csrf = Csrf::new().cookie_name("app_csrf");
  let csrf = if stored {
    csrf.store(MemoryCsrfTokenStore::new())
  } else {
    csrf
  };
  router.middleware(csrf.into_middleware());
  router.get("/login", |request: Request| async move {
    request
      .extensions()
      .get::<Session>()
      .unwrap()
      .set("user", "alice");
    "logged in"
  });
  router.post("/rotate", |request: Request| async move {
    request.extensions().get::<Session>().unwrap().rotate();
    "rotated"
  });
  router.post("/action", || async { "accepted" });
  let enter = entered.clone();
  let leave = release.clone();
  router.get("/slow", move || {
    let enter = enter.clone();
    let leave = leave.clone();
    async move {
      enter.notify_one();
      leave.notified().await;
      let mut response = Response::new(TakoBody::from("late"));
      response
        .headers_mut()
        .append("set-cookie", "preference=dark; Path=/".parse().unwrap());
      response
    }
  });
  let mut jar = BTreeMap::new();
  let login = router.dispatch(request(Method::GET, "/login", &jar)).await;
  apply_cookies(&mut jar, &login);
  let late = router.dispatch(request(Method::GET, "/slow", &jar));
  let rotate = async {
    entered.notified().await;
    let response = router
      .dispatch(request(Method::POST, "/rotate", &jar))
      .await;
    release.notify_one();
    response
  };
  let (late, rotate) = futures_util::join!(late, rotate);
  assert_eq!(rotate.status(), StatusCode::OK);
  apply_cookies(&mut jar, &rotate);
  assert_eq!(
    router
      .dispatch(request(Method::POST, "/action", &jar))
      .await
      .status(),
    StatusCode::OK
  );
  let rotated_session = jar["tako_session"].clone();
  let rotated_csrf = jar["app_csrf"].clone();
  apply_cookies(&mut jar, &late);
  assert_eq!(jar["tako_session"], rotated_session);
  assert_eq!(
    jar["preference"], "dark",
    "unrelated handler cookies must survive"
  );
  assert_eq!(
    router
      .dispatch(request(Method::POST, "/action", &jar))
      .await
      .status(),
    StatusCode::OK,
    "the late response must not invalidate the rotated session's CSRF token",
  );
  assert_eq!(jar["app_csrf"], rotated_csrf);
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn stale_response_preserves_session_bound_csrf_and_unrelated_cookies() {
  late_response_preserves_rotated_csrf(false).await;
}

#[cfg_attr(feature = "compio", compio::test)]
#[cfg_attr(not(feature = "compio"), tokio::test)]
async fn stale_response_preserves_stored_csrf_and_unrelated_cookies() {
  late_response_preserves_rotated_csrf(true).await;
}
