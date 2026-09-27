use super::*;

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
