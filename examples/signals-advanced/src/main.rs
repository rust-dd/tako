use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use tako::{
  Method,
  extractors::state::State,
  responder::Responder,
  router::Router,
  signals::{
    app_events,
    ids,
    Signal,
    SignalArbiter,
    SignalPayload,
  },
};
use tokio::net::TcpListener;
use tokio::time::{sleep, Duration};

#[derive(Debug, Clone)]
struct RequestCompletedEvent {
  method: String,
  path: String,
  status: u16,
}

impl SignalPayload for RequestCompletedEvent {
  fn id(&self) -> &'static str {
    ids::REQUEST_COMPLETED
  }

  fn to_metadata(&self) -> HashMap<String, String> {
    let mut m = HashMap::new();
    m.insert("method".into(), self.method.clone());
    m.insert("path".into(), self.path.clone());
    m.insert("status".into(), self.status.to_string());
    m
  }
}

#[derive(Debug)]
struct SlowAddRequest {
  a: i32,
  b: i32,
  delay_ms: u64,
}

#[derive(Debug, Clone)]
struct SlowAddResponse {
  sum: i32,
}

async fn hello() -> impl Responder {
  "Hello from signals-advanced".into_response()
}

async fn calc_ok(State(bus): State<SignalArbiter>) -> impl Responder {
  match bus
    .call_rpc_timeout::<SlowAddRequest, SlowAddResponse>(
      "calc.add",
      SlowAddRequest {
        a: 10,
        b: 32,
        delay_ms: 100,
      },
      Duration::from_secs(1),
    )
    .await
  {
    Ok(res) => format!("calc.add (ok) => {}", res.sum).into_response(),
    Err(err) => format!("calc.add (ok) error: {:?}", err).into_response(),
  }
}

async fn calc_timeout(State(bus): State<SignalArbiter>) -> impl Responder {
  match bus
    .call_rpc_timeout::<SlowAddRequest, SlowAddResponse>(
      "calc.add",
      SlowAddRequest {
        a: 1,
        b: 2,
        delay_ms: 1000,
      },
      Duration::from_millis(10),
    )
    .await
  {
    Ok(res) => format!("calc.add (timeout) => {}", res.sum).into_response(),
    Err(err) => format!("calc.add (timeout) error: {:?}", err).into_response(),
  }
}

async fn emit_typed(State(bus): State<SignalArbiter>) -> impl Responder {
  let event = RequestCompletedEvent {
    method: "GET".into(),
    path: "/emit-typed".into(),
    status: 201,
  };
  let sig = Signal::from_payload(&event);
  bus.emit(sig).await;

  "emitted typed RequestCompletedEvent on router bus".into_response()
}

async fn error_route() -> anyhow::Result<String> {
  Err(anyhow::anyhow!("simulated error for 5xx filter"))
}

async fn route_with_signals(State(bus): State<SignalArbiter>) -> impl Responder {
  let mut meta = HashMap::new();
  meta.insert("path".to_string(), "/route".to_string());

  bus.emit(Signal::with_metadata("routes.hit", meta)).await;

  "route_with_signals hit".into_response()
}

fn init_app_signals() {
  let bus = app_events();

  // Configure broadcast capacity
  SignalArbiter::set_global_broadcast_capacity(128);
  println!(
    "[advanced] broadcast capacity = {}",
    SignalArbiter::global_broadcast_capacity()
  );

  // Exporter: log every signal as a simple line
  bus.register_exporter(|signal: &Signal| {
    println!("[advanced][exporter] {} {:?}", signal.id, signal.metadata);
  });

  // Log server start
  bus.on(ids::SERVER_STARTED, |signal: Signal| async move {
    println!("[advanced][server] started: {:?}", signal.metadata);
  });

  // Simple handler for completed requests
  bus.on(ids::REQUEST_COMPLETED, |signal: Signal| async move {
    println!("[advanced][handler] {} -> {:?}", signal.id, signal.metadata);
  });

  // RPC error events
  bus.on(ids::RPC_ERROR, |signal: Signal| async move {
    println!("[advanced][rpc_error] {:?}", signal.metadata);
  });

  // Prefix subscription for all request.* signals
  let mut prefix_rx = bus.subscribe_prefix("request.");
  tokio::spawn(async move {
    while let Ok(signal) = prefix_rx.recv().await {
      println!("[advanced][prefix] {} {:?}", signal.id, signal.metadata);
    }
  });

  // Filtered subscription for 5xx request.completed signals
  let filtered_bus = bus.clone();
  let mut error_stream = filtered_bus.subscribe_filtered(ids::REQUEST_COMPLETED, |sig: &Signal| {
    sig
      .metadata
      .get("status")
      .and_then(|s| s.parse::<u16>().ok())
      .map(|status| status >= 500)
      .unwrap_or(false)
  });

  tokio::spawn(async move {
    while let Some(signal) = error_stream.recv().await {
      println!("[advanced][filtered] error signal: {:?}", signal.metadata);
    }
  });

  // Introspection for app-level topics
  println!("[advanced][app] signal ids: {:?}", bus.signal_ids());
  println!("[advanced][app] signal prefixes: {:?}", bus.signal_prefixes());
}

fn init_router_signals(router: &mut Router) {
  let bus = router.signal_arbiter();

  // Expose router-level arbiter to handlers
  router.state(bus.clone());

  // Route-level event logging
  router.on_signal("routes.hit", |signal: Signal| async move {
    if let Some(path) = signal.metadata.get("path") {
      println!("[advanced][router] routes.hit for path: {}", path);
    }
  });

  // Typed RPC on router-level bus
  bus.register_rpc::<SlowAddRequest, SlowAddResponse, _, _>(
    "calc.add",
    |req: Arc<SlowAddRequest>| async move {
      sleep(Duration::from_millis(req.delay_ms)).await;
      SlowAddResponse { sum: req.a + req.b }
    },
  );

  println!("[advanced][router] rpc ids: {:?}", bus.rpc_ids());
}

#[tokio::main]
async fn main() -> Result<()> {
  // Initialize advanced app-level signal listeners
  init_app_signals();

  let listener = TcpListener::bind("127.0.0.1:8082").await?;

  let mut router = Router::new();
  init_router_signals(&mut router);

  router.route(Method::GET, "/", hello);
  router.route(Method::GET, "/calc/ok", calc_ok);
  router.route(Method::GET, "/calc/timeout", calc_timeout);
  router.route(Method::GET, "/emit-typed", emit_typed);

  // Route with its own route-level signal handlers
  let route = router.route(Method::GET, "/route", route_with_signals);
  route.on_signal(ids::ROUTE_REQUEST_COMPLETED, |signal: Signal| async move {
    println!("[advanced][route-level] /route completed: {:?}", signal.metadata);
  });

  router.route(Method::GET, "/error", error_route);

  tako::serve(listener, router).await;

  Ok(())
}
