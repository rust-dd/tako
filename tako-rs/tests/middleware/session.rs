use super::*;

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn session_new_request_sets_cookie() {
  use tako::middleware::session::SessionMiddleware;

  let mut router = Router::new();
  router.route(Method::GET, "/", |_req: Request| async { "ok" });
  router.middleware(SessionMiddleware::new().into_middleware());

  let resp = router.dispatch(make_req(Method::GET, "/")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  let set_cookie = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .find(|v| v.to_str().unwrap().starts_with("tako_session="));
  assert!(set_cookie.is_some(), "session cookie should be set");
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn session_get_set_data() {
  use tako::middleware::session::Session;
  use tako::middleware::session::SessionMiddleware;

  let mut router = Router::new();
  router.route(Method::POST, "/set", |req: Request| async move {
    let session = req.extensions().get::<Session>().unwrap();
    session.set("counter", 42u32);
    "stored"
  });
  router.route(Method::GET, "/get", |req: Request| async move {
    let session = req.extensions().get::<Session>().unwrap();
    let val: Option<u32> = session.get("counter");
    format!("counter={}", val.unwrap_or(0))
  });
  router.middleware(SessionMiddleware::new().into_middleware());

  let resp = router.dispatch(make_req(Method::POST, "/set")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  let cookie = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .find(|v| v.to_str().unwrap().starts_with("tako_session="))
    .unwrap()
    .to_str()
    .unwrap()
    .to_string();
  let session_id = cookie.split('=').nth(1).unwrap().split(';').next().unwrap();

  let mut req = make_req(Method::GET, "/get");
  req.headers_mut().insert(
    "cookie",
    format!("tako_session={session_id}").parse().unwrap(),
  );

  let resp = router.dispatch(req).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "counter=42");
}

#[cfg_attr(not(feature = "compio"), tokio::test)]
#[cfg_attr(feature = "compio", compio::test)]
async fn session_destroy_expires_cookie() {
  use tako::middleware::session::Session;
  use tako::middleware::session::SessionMiddleware;

  let mut router = Router::new();
  router.route(Method::POST, "/logout", |req: Request| async move {
    let session = req.extensions().get::<Session>().unwrap();
    session.destroy();
    "bye"
  });
  router.middleware(SessionMiddleware::new().into_middleware());

  let first = router
    .dispatch(make_req(Method::GET, "/dontcare"))
    .await
    .headers()
    .get_all("set-cookie")
    .iter()
    .filter_map(|v| v.to_str().ok())
    .find(|s| s.starts_with("tako_session="))
    .map(str::to_owned);
  let inbound = first.expect("first response should set cookie");
  let sid = inbound
    .split('=')
    .nth(1)
    .unwrap()
    .split(';')
    .next()
    .unwrap();

  let mut req = make_req(Method::POST, "/logout");
  req
    .headers_mut()
    .insert("cookie", format!("tako_session={sid}").parse().unwrap());
  let resp = router.dispatch(req).await;

  let expired = resp
    .headers()
    .get_all("set-cookie")
    .iter()
    .filter_map(|v| v.to_str().ok())
    .find(|s| s.starts_with("tako_session="))
    .expect("logout response should carry expiring cookie");
  assert!(
    expired.contains("Max-Age=0"),
    "expected Max-Age=0, got: {expired}"
  );
  assert!(
    expired.contains("Expires=Thu, 01 Jan 1970"),
    "expected past Expires, got: {expired}"
  );
}
