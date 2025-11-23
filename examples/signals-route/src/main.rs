use std::collections::HashMap;
use tako::types::BuildHasher;

use anyhow::Result;
use tako::{
  Method,
  extractors::state::State,
  responder::Responder,
  router::Router,
  signals::{Signal, SignalArbiter},
};
use tokio::net::TcpListener;

async fn route_handler(State(bus): State<SignalArbiter>) -> impl Responder {
  let mut meta: HashMap<String, String, BuildHasher> = HashMap::with_hasher(BuildHasher::default());
  // In a real app you might extract the path from Request via another extractor;
  // here we just tag a static value for demonstration.
  meta.insert("path".to_string(), "/route".to_string());

  // Emit a route-level event through the router's arbiter
  bus.emit(Signal::with_metadata("routes.hit", meta)).await;

  "Route-level signals for /route".into_response()
}

fn init_route_signals(router: &mut Router) {
  // Expose the router-level arbiter to handlers via State<SignalArbiter>
  let arbiter = router.signal_arbiter();
  router.state(arbiter.clone());

  // Log all route-level hits
  router.on_signal("routes.hit", |signal: Signal| async move {
    if let Some(path) = signal.metadata.get("path") {
      println!("[signals-route] routes.hit for path: {}", path);
    }
  });
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  init_route_signals(&mut router);

  router.route(Method::GET, "/route", route_handler);

  tako::serve(listener, router).await;

  Ok(())
}
