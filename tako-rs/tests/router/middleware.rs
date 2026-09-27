use super::*;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MiddlewareStage(u8);

#[tokio::test]
async fn global_and_route_middlewares_preserve_order_and_extensions() {
  let mut router = Router::new();
  let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));

  router.middleware({
    let events = Arc::clone(&events);
    move |mut req: Request, next: tako::middleware::Next| {
      let events = Arc::clone(&events);
      async move {
        events.lock().unwrap().push("global-before");
        req.extensions_mut().insert(MiddlewareStage(1));

        let mut resp = next.run(req).await;
        events.lock().unwrap().push("global-after");
        resp
          .headers_mut()
          .insert("x-global", "applied".parse().unwrap());
        resp
      }
    }
  });

  let route_events = Arc::clone(&events);
  let route = router.route(Method::GET, "/hello", move |req: Request| {
    let route_events = Arc::clone(&route_events);
    async move {
      route_events.lock().unwrap().push("handler");
      assert_eq!(
        req.extensions().get::<MiddlewareStage>(),
        Some(&MiddlewareStage(2))
      );
      "ok"
    }
  });

  route.middleware({
    let events = Arc::clone(&events);
    move |mut req: Request, next: tako::middleware::Next| {
      let events = Arc::clone(&events);
      async move {
        events.lock().unwrap().push("route-before");
        assert_eq!(
          req.extensions().get::<MiddlewareStage>(),
          Some(&MiddlewareStage(1))
        );
        req.extensions_mut().insert(MiddlewareStage(2));

        let mut resp = next.run(req).await;
        events.lock().unwrap().push("route-after");
        resp
          .headers_mut()
          .insert("x-route", "applied".parse().unwrap());
        resp
      }
    }
  });

  let resp = router.dispatch(make_req(Method::GET, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(resp.headers().get("x-global").unwrap(), "applied");
  assert_eq!(resp.headers().get("x-route").unwrap(), "applied");
  assert_eq!(body_str(resp).await, "ok");
  assert_eq!(
    events.lock().unwrap().as_slice(),
    &[
      "global-before",
      "route-before",
      "handler",
      "route-after",
      "global-after",
    ]
  );
}

#[tokio::test]
async fn global_middleware_wraps_fallback() {
  let mut router = Router::new();
  router.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp
      .headers_mut()
      .insert("x-global-fallback", "applied".parse().unwrap());
    resp
  });
  router.fallback(|_req: Request| async { (StatusCode::NOT_FOUND, "missing") });

  let resp = router.dispatch(make_req(Method::GET, "/missing")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
  assert_eq!(resp.headers().get("x-global-fallback").unwrap(), "applied");
  assert_eq!(body_str(resp).await, "missing");
}

#[tokio::test]
async fn global_middleware_wraps_tsr_redirect() {
  let mut router = Router::new();
  router.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp
      .headers_mut()
      .insert("x-global-tsr", "applied".parse().unwrap());
    resp
  });
  router.route_with_tsr(Method::GET, "/api", |_req: Request| async { "API" });

  let resp = router.dispatch(make_req(Method::GET, "/api/")).await;
  assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
  assert_eq!(resp.headers().get("location").unwrap(), "/api");
  assert_eq!(resp.headers().get("x-global-tsr").unwrap(), "applied");
}

#[tokio::test]
async fn merge_preserves_middleware_order_on_merged_routes() {
  let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));

  let mut sub = Router::new();
  sub.middleware({
    let events = Arc::clone(&events);
    move |req: Request, next: tako::middleware::Next| {
      let events = Arc::clone(&events);
      async move {
        events.lock().unwrap().push("sub-global-before");
        let mut resp = next.run(req).await;
        events.lock().unwrap().push("sub-global-after");
        resp
          .headers_mut()
          .insert("x-sub-global", "applied".parse().unwrap());
        resp
      }
    }
  });
  let sub_events = Arc::clone(&events);
  let route = sub.route(Method::GET, "/sub", move |_req: Request| {
    let sub_events = Arc::clone(&sub_events);
    async move {
      sub_events.lock().unwrap().push("handler");
      "sub"
    }
  });
  route.middleware({
    let events = Arc::clone(&events);
    move |req: Request, next: tako::middleware::Next| {
      let events = Arc::clone(&events);
      async move {
        events.lock().unwrap().push("route-before");
        let mut resp = next.run(req).await;
        events.lock().unwrap().push("route-after");
        resp
          .headers_mut()
          .insert("x-route", "applied".parse().unwrap());
        resp
      }
    }
  });

  let mut main = Router::new();
  main.middleware({
    let events = Arc::clone(&events);
    move |req: Request, next: tako::middleware::Next| {
      let events = Arc::clone(&events);
      async move {
        events.lock().unwrap().push("main-global-before");
        let mut resp = next.run(req).await;
        events.lock().unwrap().push("main-global-after");
        resp
          .headers_mut()
          .insert("x-main-global", "applied".parse().unwrap());
        resp
      }
    }
  });
  main.merge(sub);

  let resp = main.dispatch(make_req(Method::GET, "/sub")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(resp.headers().get("x-main-global").unwrap(), "applied");
  assert_eq!(resp.headers().get("x-sub-global").unwrap(), "applied");
  assert_eq!(resp.headers().get("x-route").unwrap(), "applied");
  assert_eq!(body_str(resp).await, "sub");
  assert_eq!(
    events.lock().unwrap().as_slice(),
    &[
      "main-global-before",
      "sub-global-before",
      "route-before",
      "handler",
      "route-after",
      "sub-global-after",
      "main-global-after",
    ]
  );
}
