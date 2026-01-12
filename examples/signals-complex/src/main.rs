use std::collections::HashMap;
use std::sync::Arc;

use anyhow::Result;
use tako::Method;
use tako::extractors::state::State;
use tako::responder::Responder;
use tako::router::Router;
use tako::signals::Signal;
use tako::signals::SignalArbiter;
use tako::signals::app_events;
use tako::signals::ids;
use tako::types::BuildHasher;
use tokio::net::TcpListener;
use tokio::time::Duration;
use tokio::time::sleep;

#[derive(Debug)]
struct AddRequest {
  a: i32,
  b: i32,
}

#[derive(Debug, Clone)]
struct AddResponse {
  sum: i32,
}

async fn hello() -> impl Responder {
  "Hello from signals-complex".into_response()
}

async fn calc(State(arbiter): State<SignalArbiter>) -> impl Responder {
  if let Some(res) = arbiter
    .call_rpc::<AddRequest, AddResponse>("calc.add", AddRequest { a: 10, b: 32 })
    .await
  {
    format!("calc.add => {}", res.sum).into_response()
  } else {
    "calc.add RPC failed".into_response()
  }
}

async fn trigger_hot_reload() -> impl Responder {
  let mut meta: HashMap<String, String, BuildHasher> = HashMap::with_hasher(BuildHasher::default());
  meta.insert("reason".to_string(), "manual-trigger".to_string());
  SignalArbiter::emit_app(Signal::with_metadata(ids::ROUTER_HOT_RELOAD, meta)).await;

  "router.hot_reload emitted".into_response()
}

fn init_app_signals() {
  let arbiter = app_events();

  // Log when server starts
  arbiter.on(ids::SERVER_STARTED, |signal: Signal| async move {
    println!("[signals-complex] server.started: {:?}", signal.metadata);
  });

  // Metrics-style listener for completed requests
  let mut rx = arbiter.subscribe(ids::REQUEST_COMPLETED);
  tokio::spawn(async move {
    while let Ok(signal) = rx.recv().await {
      let method = signal.metadata.get("method").cloned().unwrap_or_default();
      let path = signal.metadata.get("path").cloned().unwrap_or_default();
      let status = signal.metadata.get("status").cloned().unwrap_or_default();
      println!(
        "[signals-complex] request.completed: {} {} -> {}",
        method, path, status
      );
    }
  });

  // Wait once for router.hot_reload, then log
  let arbiter_once = app_events().clone();
  tokio::spawn(async move {
    if let Some(signal) = arbiter_once.once(ids::ROUTER_HOT_RELOAD).await {
      println!("[signals-complex] router.hot_reload: {:?}", signal.metadata);
    }
  });
}

fn init_router_signals(router: &mut Router) {
  let arbiter = router.signal_arbiter();

  // Expose router-level arbiter to handlers
  router.state(arbiter.clone());

  // Route-level event logging
  router.on_signal("routes.hit", |signal: Signal| async move {
    if let Some(path) = signal.metadata.get("path") {
      println!("[signals-complex] routes.hit for path: {}", path);
    }
  });

  // Typed RPC on router-level arbiter
  arbiter
    .register_rpc::<AddRequest, AddResponse, _, _>("calc.add", |req: Arc<AddRequest>| async move {
      AddResponse { sum: req.a + req.b }
    });
}

async fn route_with_signals(State(arbiter): State<SignalArbiter>) -> impl Responder {
  let mut meta: HashMap<String, String, BuildHasher> = HashMap::with_hasher(BuildHasher::default());
  meta.insert("path".to_string(), "/route".to_string());

  arbiter
    .emit(Signal::with_metadata("routes.hit", meta))
    .await;

  "route_with_signals hit".into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  init_app_signals();

  let listener = TcpListener::bind("127.0.0.1:8081").await?;

  let mut router = Router::new();
  init_router_signals(&mut router);

  router.route(Method::GET, "/", hello);
  router.route(Method::GET, "/calc", calc);
  router.route(Method::GET, "/route", route_with_signals);
  router.route(Method::GET, "/hot-reload", trigger_hot_reload);

  // Let the server run; you can hit the endpoints from a browser or curl
  tokio::spawn(async move {
    tako::serve(listener, router).await;
  });

  // Keep the example alive for a bit for demonstration
  sleep(Duration::from_secs(300)).await;

  Ok(())
}
