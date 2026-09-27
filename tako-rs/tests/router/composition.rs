use super::*;

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

#[derive(Clone)]
struct GlobalOnly(&'static str);

async fn read_global(
  tako::extractors::state::State(g): tako::extractors::state::State<GlobalOnly>,
) -> impl tako::responder::Responder {
  g.0
}

#[tokio::test]
async fn with_state_falls_back_to_global_when_unset_per_router() {
  // A router that never called `with_state::<T>` should still see the global
  // value installed via `set_state` — backward-compat guarantee.
  use tako::state::set_state;

  set_state(GlobalOnly("global"));

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

#[test]
fn routes_is_empty_for_a_new_router() {
  let router = Router::new();
  assert!(router.routes().is_empty());
}

#[test]
fn routes_returns_every_registered_route() {
  let mut router = Router::new();
  router.route(Method::GET, "/health", |_req: Request| async { "ok" });
  router.route(Method::POST, "/shorten", |_req: Request| async { "ok" });
  router.route(Method::GET, "/{id}", |_req: Request| async { "ok" });

  let mut seen: Vec<(Method, String)> = router
    .routes()
    .iter()
    .map(|r| (r.method.clone(), r.path.to_string()))
    .collect();
  seen.sort_by(|a, b| a.1.cmp(&b.1));

  assert_eq!(
    seen,
    vec![
      (Method::GET, "/health".to_string()),
      (Method::POST, "/shorten".to_string()),
      (Method::GET, "/{id}".to_string()),
    ]
  );
}

#[test]
fn routes_preserves_the_tsr_flag() {
  let mut router = Router::new();
  router.route(Method::GET, "/plain", |_req: Request| async { "ok" });
  router.route_with_tsr(Method::GET, "/tsr", |_req: Request| async { "ok" });

  let routes = router.routes();
  let plain = routes.iter().find(|r| r.path.as_ref() == "/plain").unwrap();
  let tsr = routes.iter().find(|r| r.path.as_ref() == "/tsr").unwrap();
  assert!(!plain.tsr);
  assert!(tsr.tsr);
}

#[test]
fn routes_reflect_scope_prefixes() {
  let mut router = Router::new();
  router.scope("/api/v1", |r| {
    r.get("/users", |_req: Request| async { "ok" });
  });

  let paths: Vec<String> = router.routes().iter().map(|r| r.path.to_string()).collect();
  assert_eq!(paths, vec!["/api/v1/users".to_string()]);
}
