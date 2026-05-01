use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;

use http::Method;
use http::StatusCode;
use http_body_util::BodyExt;
use tako::body::TakoBody;
#[cfg(feature = "plugins")]
use tako::plugins::TakoPlugin;
use tako::router::Router;
#[cfg(feature = "plugins")]
use tako::router::Router as TakoPluginRouter;
use tako::types::Request;

fn make_req(method: Method, uri: &str) -> Request {
  http::Request::builder()
    .method(method)
    .uri(uri)
    .body(TakoBody::empty())
    .unwrap()
}

async fn body_str(resp: tako::types::Response) -> String {
  let bytes = resp.into_body().collect().await.unwrap().to_bytes();
  String::from_utf8(bytes.to_vec()).unwrap()
}

#[tokio::test]
async fn route_match_returns_200() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });

  let resp = router.dispatch(make_req(Method::GET, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "Hello");
}

#[tokio::test]
async fn route_miss_returns_404() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });

  let resp = router.dispatch(make_req(Method::GET, "/notfound")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn different_method_returns_405_with_allow() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });

  let resp = router.dispatch(make_req(Method::POST, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
  let allow = resp
    .headers()
    .get(http::header::ALLOW)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("");
  assert!(allow.split(',').map(str::trim).any(|m| m == "GET"));
}

#[tokio::test]
async fn custom_fallback() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });
  router.fallback(|_req: Request| async { (StatusCode::NOT_FOUND, "Custom 404") });

  let resp = router.dispatch(make_req(Method::GET, "/nope")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
  assert_eq!(body_str(resp).await, "Custom 404");
}

#[tokio::test]
async fn tsr_redirect() {
  let mut router = Router::new();
  router.route_with_tsr(Method::GET, "/api", |_req: Request| async { "API" });

  // Exact match
  let resp = router.dispatch(make_req(Method::GET, "/api")).await;
  assert_eq!(resp.status(), StatusCode::OK);

  // Trailing slash → 307 redirect
  let resp = router.dispatch(make_req(Method::GET, "/api/")).await;
  assert_eq!(resp.status(), StatusCode::TEMPORARY_REDIRECT);
  assert_eq!(resp.headers().get("location").unwrap(), "/api");
}

#[tokio::test]
#[should_panic(expected = "Cannot route with TSR for root path")]
async fn tsr_root_panics() {
  let mut router = Router::new();
  router.route_with_tsr(Method::GET, "/", |_req: Request| async { "root" });
}

#[tokio::test]
async fn global_middleware_runs() {
  let mut router = Router::new();
  router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });
  router.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp
      .headers_mut()
      .insert("x-middleware", "applied".parse().unwrap());
    resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(resp.headers().get("x-middleware").unwrap(), "applied");
}

#[tokio::test]
async fn router_timeout_returns_408() {
  let mut router = Router::new();
  router.timeout(Duration::from_millis(10));
  router.route(Method::GET, "/slow", |_req: Request| async {
    tokio::time::sleep(Duration::from_millis(100)).await;
    "done"
  });

  let resp = router.dispatch(make_req(Method::GET, "/slow")).await;
  assert_eq!(resp.status(), StatusCode::REQUEST_TIMEOUT);
}

#[tokio::test]
async fn router_timeout_fallback() {
  let mut router = Router::new();
  router.timeout(Duration::from_millis(10));
  router.timeout_fallback(|_req: Request| async { (StatusCode::GATEWAY_TIMEOUT, "Too slow!") });
  router.route(Method::GET, "/slow", |_req: Request| async {
    tokio::time::sleep(Duration::from_millis(100)).await;
    "done"
  });

  let resp = router.dispatch(make_req(Method::GET, "/slow")).await;
  assert_eq!(resp.status(), StatusCode::GATEWAY_TIMEOUT);
  assert_eq!(body_str(resp).await, "Too slow!");
}

#[tokio::test]
async fn error_handler_transforms_5xx() {
  let mut router = Router::new();
  router.route(Method::GET, "/error", |_req: Request| async {
    (StatusCode::INTERNAL_SERVER_ERROR, "oops")
  });
  router.error_handler(|resp| {
    let status = resp.status();
    let mut new_resp = http::Response::new(TakoBody::from(format!(
      "{{\"error\":\"{}\"}}",
      status.as_u16()
    )));
    *new_resp.status_mut() = status;
    new_resp.headers_mut().insert(
      http::header::CONTENT_TYPE,
      "application/json".parse().unwrap(),
    );
    new_resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/error")).await;
  assert_eq!(resp.status(), StatusCode::INTERNAL_SERVER_ERROR);
  assert_eq!(
    resp.headers().get("content-type").unwrap(),
    "application/json"
  );
  assert_eq!(body_str(resp).await, "{\"error\":\"500\"}");
}

#[tokio::test]
async fn error_handler_ignores_non_5xx() {
  let mut router = Router::new();
  router.route(Method::GET, "/ok", |_req: Request| async { "ok" });
  router.error_handler(|_resp| {
    let mut r = http::Response::new(TakoBody::from("transformed"));
    *r.status_mut() = StatusCode::INTERNAL_SERVER_ERROR;
    r
  });

  let resp = router.dispatch(make_req(Method::GET, "/ok")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "ok");
}

#[tokio::test]
async fn merge_routers() {
  let mut sub = Router::new();
  sub.route(Method::GET, "/sub", |_req: Request| async { "sub" });

  let mut main = Router::new();
  main.route(Method::GET, "/main", |_req: Request| async { "main" });
  main.merge(sub);

  let resp = main.dispatch(make_req(Method::GET, "/main")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "main");

  let resp = main.dispatch(make_req(Method::GET, "/sub")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "sub");
}

#[tokio::test]
async fn multiple_routes_different_methods() {
  let mut router = Router::new();
  router.route(Method::GET, "/item", |_req: Request| async { "get" });
  router.route(Method::POST, "/item", |_req: Request| async { "post" });

  let resp_get = router.dispatch(make_req(Method::GET, "/item")).await;
  assert_eq!(resp_get.status(), StatusCode::OK);
  assert_eq!(body_str(resp_get).await, "get");

  let resp_post = router.dispatch(make_req(Method::POST, "/item")).await;
  assert_eq!(resp_post.status(), StatusCode::OK);
  assert_eq!(body_str(resp_post).await, "post");
}

#[tokio::test]
async fn route_level_middleware_runs() {
  let mut router = Router::new();
  let route = router.route(Method::GET, "/hello", |_req: Request| async { "Hello" });
  route.middleware(|req: Request, next: tako::middleware::Next| async move {
    let mut resp = next.run(req).await;
    resp
      .headers_mut()
      .insert("x-route-middleware", "applied".parse().unwrap());
    resp
  });

  let resp = router.dispatch(make_req(Method::GET, "/hello")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(resp.headers().get("x-route-middleware").unwrap(), "applied");
}

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

#[cfg(feature = "plugins")]
#[derive(Clone)]
struct TestPlugin {
  events: Arc<Mutex<Vec<&'static str>>>,
}

#[cfg(feature = "plugins")]
impl TakoPlugin for TestPlugin {
  fn name(&self) -> &'static str {
    "test-plugin"
  }

  fn setup(&self, router: &TakoPluginRouter) -> anyhow::Result<()> {
    let events = Arc::clone(&self.events);
    router.middleware(move |req: Request, next: tako::middleware::Next| {
      let events = Arc::clone(&events);
      async move {
        events.lock().unwrap().push("plugin-before");
        let mut resp = next.run(req).await;
        events.lock().unwrap().push("plugin-after");
        resp
          .headers_mut()
          .insert("x-plugin", "applied".parse().unwrap());
        resp
      }
    });
    Ok(())
  }
}

#[cfg(feature = "plugins")]
#[tokio::test]
async fn route_plugin_runs_once_and_precedes_route_middleware() {
  let mut router = Router::new();
  let events = Arc::new(Mutex::new(Vec::<&'static str>::new()));

  let handler_events = Arc::clone(&events);
  let route = router.route(Method::GET, "/plugin", move |_req: Request| {
    let handler_events = Arc::clone(&handler_events);
    async move {
      handler_events.lock().unwrap().push("handler");
      "ok"
    }
  });
  route.plugin(TestPlugin {
    events: Arc::clone(&events),
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

  let resp1 = router.dispatch(make_req(Method::GET, "/plugin")).await;
  assert_eq!(resp1.status(), StatusCode::OK);
  assert_eq!(resp1.headers().get("x-plugin").unwrap(), "applied");
  assert_eq!(resp1.headers().get("x-route").unwrap(), "applied");
  assert_eq!(body_str(resp1).await, "ok");

  let resp2 = router.dispatch(make_req(Method::GET, "/plugin")).await;
  assert_eq!(resp2.status(), StatusCode::OK);
  assert_eq!(resp2.headers().get("x-plugin").unwrap(), "applied");
  assert_eq!(resp2.headers().get("x-route").unwrap(), "applied");
  assert_eq!(body_str(resp2).await, "ok");

  assert_eq!(
    events.lock().unwrap().as_slice(),
    &[
      "plugin-before",
      "route-before",
      "handler",
      "route-after",
      "plugin-after",
      "plugin-before",
      "route-before",
      "handler",
      "route-after",
      "plugin-after",
    ]
  );
}

#[tokio::test]
async fn nest_mounts_child_under_prefix() {
  let mut child = Router::new();
  child.get("/users", |_req: Request| async { "users" });
  child.post("/users", |_req: Request| async { "created" });

  let mut root = Router::new();
  root.nest("/api/v1", child);

  let resp = root.dispatch(make_req(Method::GET, "/api/v1/users")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "users");

  let resp = root.dispatch(make_req(Method::POST, "/api/v1/users")).await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "created");

  // Original (unprefixed) child path is not registered on the root.
  let resp = root.dispatch(make_req(Method::GET, "/users")).await;
  assert_eq!(resp.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn nest_does_not_double_stack_middleware_on_re_nest() {
  // Re-using the same child router across two nest calls must not stack the
  // child's global middleware twice on the same Arc<Route> (the bug Router::merge has).
  let counter = Arc::new(Mutex::new(0u32));
  let counter_mw = counter.clone();

  let mut child = Router::new();
  child.get("/ping", |_req: Request| async { "pong" });
  child.middleware(move |req, next| {
    let counter = counter_mw.clone();
    async move {
      *counter.lock().unwrap() += 1;
      next.run(req).await
    }
  });

  let mut root = Router::new();
  // Nest the same logical child twice under different prefixes — middleware
  // must only fire once per request, not twice.
  // Build a second child with the same shape because Router is move-only.
  let mut child2 = Router::new();
  child2.get("/ping", |_req: Request| async { "pong" });
  let counter_mw2 = counter.clone();
  child2.middleware(move |req, next| {
    let counter = counter_mw2.clone();
    async move {
      *counter.lock().unwrap() += 1;
      next.run(req).await
    }
  });

  root.nest("/a", child);
  root.nest("/b", child2);

  let _ = root.dispatch(make_req(Method::GET, "/a/ping")).await;
  assert_eq!(
    *counter.lock().unwrap(),
    1,
    "middleware fired more than once on /a/ping"
  );
  let _ = root.dispatch(make_req(Method::GET, "/b/ping")).await;
  assert_eq!(
    *counter.lock().unwrap(),
    2,
    "middleware fired more than once on /b/ping"
  );
}

#[tokio::test]
async fn with_state_isolates_two_routers_in_same_process() {
  // Each router holds its own `String` state, distinct from the other and
  // from any process-global value. The previous global-only `set_state` API
  // could not express this without newtype wrappers.
  use tako::extractors::state::State;

  async fn echo_state(State(s): State<String>) -> impl tako::responder::Responder {
    (*s).clone()
  }

  let mut router_a = Router::new();
  router_a.with_state::<String>("router-a".to_string());
  router_a.get("/whoami", echo_state);

  let mut router_b = Router::new();
  router_b.with_state::<String>("router-b".to_string());
  router_b.get("/whoami", echo_state);

  let resp_a = router_a.dispatch(make_req(Method::GET, "/whoami")).await;
  assert_eq!(body_str(resp_a).await, "router-a");

  let resp_b = router_b.dispatch(make_req(Method::GET, "/whoami")).await;
  assert_eq!(body_str(resp_b).await, "router-b");
}

#[tokio::test]
async fn with_state_falls_back_to_global_when_unset_per_router() {
  // A router that never called `with_state::<T>` should still see the global
  // value installed via `set_state` — backward-compat guarantee.
  use tako::extractors::state::State;
  use tako::state::set_state;

  #[derive(Clone)]
  struct GlobalOnly(&'static str);

  set_state(GlobalOnly("global"));

  async fn read_global(State(g): State<GlobalOnly>) -> impl tako::responder::Responder {
    g.0
  }

  let mut router = Router::new();
  router.get("/g", read_global);

  let resp = router.dispatch(make_req(Method::GET, "/g")).await;
  assert_eq!(body_str(resp).await, "global");
}

#[tokio::test]
async fn scope_groups_routes_under_prefix() {
  let mut router = Router::new();
  router.scope("/api/v1", |r| {
    r.get("/users", |_req: Request| async { "users" });
    r.scope("/admin", |r2| {
      r2.get("/dashboard", |_req: Request| async { "dashboard" });
    });
  });

  let resp = router
    .dispatch(make_req(Method::GET, "/api/v1/users"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
  let resp = router
    .dispatch(make_req(Method::GET, "/api/v1/admin/dashboard"))
    .await;
  assert_eq!(resp.status(), StatusCode::OK);
  assert_eq!(body_str(resp).await, "dashboard");
}
