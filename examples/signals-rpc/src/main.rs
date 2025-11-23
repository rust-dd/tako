use std::sync::Arc;

use anyhow::Result;
use tako::signals::{Signal, SignalArbiter};
use tokio::time::{Duration, sleep};
use tako::types::BuildHasher;

#[derive(Debug)]
struct AddRequest {
  a: i32,
  b: i32,
}

#[derive(Debug, Clone)]
struct AddResponse {
  sum: i32,
}

#[tokio::main]
async fn main() -> Result<()> {
  let arbiter = SignalArbiter::new();

  // Register a typed RPC handler under "rpc.add"
  arbiter
    .register_rpc::<AddRequest, AddResponse, _, _>("rpc.add", |req: Arc<AddRequest>| async move {
      AddResponse { sum: req.a + req.b }
    });

  // Call the RPC handler and print the result
  if let Some(res) = arbiter
    .call_rpc::<AddRequest, AddResponse>("rpc.add", AddRequest { a: 2, b: 40 })
    .await
  {
    println!("[signals-rpc] 2 + 40 = {}", res.sum);
  }

  // Demonstrate waiting for a specific event once
  let arbiter_for_listener = arbiter.clone();
  tokio::spawn(async move {
    if let Some(signal) = arbiter_for_listener.once("custom.event").await {
      println!("[signals-rpc] received custom.event: {:?}", signal.metadata);
    }
  });

  // Emit a custom event that the listener above will receive
  let mut meta: std::collections::HashMap<String, String, BuildHasher> = std::collections::HashMap::with_hasher(BuildHasher::default());
  meta.insert("message".to_string(), "hello from signals-rpc".to_string());
  arbiter
    .emit(Signal::with_metadata("custom.event", meta))
    .await;

  // Give the listener a moment to process the event before exiting
  sleep(Duration::from_millis(100)).await;

  Ok(())
}
