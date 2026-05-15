# Signals

Signals are Tako's in-process pub/sub bus for framework-internal events
and application-level RPC. The framework emits well-known signals on
every request, connection, and queue job; you subscribe to the IDs
that matter.

Signal IDs emitted by `tako::signals::ids` are part of the stable
contract — operators write dashboards and alerts against them, so
renaming an ID is a major-version event. See
[API stability](../reference/stability.md) for the full list.

```rust
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;
use tako::signals::{Signal, app_events, ids};
use tako::types::Request;

async fn hello(_: Request) -> impl Responder { "hi" }

fn install_listeners() {
  let arbiter = app_events();

  // One-shot callback when the server starts up.
  arbiter.on(ids::SERVER_STARTED, |sig: Signal| async move {
    println!("server.started: {:?}", sig.metadata);
  });

  // Long-running listener that logs every completed request.
  let mut rx = arbiter.subscribe(ids::REQUEST_COMPLETED);
  tokio::spawn(async move {
    while let Ok(sig) = rx.recv().await {
      let method = sig.metadata.get("method").cloned().unwrap_or_default();
      let path   = sig.metadata.get("path").cloned().unwrap_or_default();
      let status = sig.metadata.get("status").cloned().unwrap_or_default();
      println!("request.completed: {method} {path} -> {status}");
    }
  });
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  install_listeners();

  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
  let mut router = Router::new();
  router.route(Method::GET, "/", hello);
  tako::serve(listener, router).await;
  Ok(())
}
```

`app_events()` returns the process-wide `&'static SignalArbiter`.
Per-router and connection-level signals (`request.started`,
`request.completed`, `connection.opened`, `connection.closed`,
`server.started`, plus per-route `route.request.*`) are emitted from
a single site (`Router::dispatch` + the transport helpers in
`tako::signals::transport`), so every transport gets the same
payload for free.

A router can have its own arbiter via `router.signal_arbiter()` —
useful when you want application-level events to flow through the
same fan-out infrastructure without colliding with the global bus.

## RPC over signals

The same arbiter doubles as a typed RPC bus:

```rust
use std::sync::Arc;
use tako::signals::SignalArbiter;

#[derive(Debug)]
struct AddRequest { a: i32, b: i32 }
#[derive(Debug, Clone)]
struct AddResponse { sum: i32 }

#[tokio::main]
async fn main() {
  let arbiter = SignalArbiter::new();

  arbiter.register_rpc::<AddRequest, AddResponse, _, _>(
    "rpc.add",
    |req: Arc<AddRequest>| async move { AddResponse { sum: req.a + req.b } },
  );

  let res = arbiter.call_rpc::<AddRequest, AddResponse>(
    "rpc.add",
    AddRequest { a: 2, b: 40 },
  ).await.unwrap();
  assert_eq!(res.sum, 42);
}
```

`register_rpc` installs a typed handler; `call_rpc` /
`call_rpc_timeout` / `call_rpc_result` invoke it. Errors are reported
via the `RpcError` enum.

## Cluster-wide forwarding

The `signals::bus::SignalBus` async trait is the integration point
for cluster-wide forwarding. The default `LocalBus` is a no-op;
companion crates can implement Redis pub/sub, NATS, or Kafka
backends without changing application code.

See:

- [`examples/signals-basic`](https://github.com/rust-dd/tako/tree/main/examples/signals-basic)
  — subscribe to `request.completed`,
- [`examples/signals-route`](https://github.com/rust-dd/tako/tree/main/examples/signals-route)
  — per-router arbiter and custom IDs,
- [`examples/signals-rpc`](https://github.com/rust-dd/tako/tree/main/examples/signals-rpc)
  — typed RPC handlers,
- [`examples/signals-advanced`](https://github.com/rust-dd/tako/tree/main/examples/signals-advanced)
  and [`examples/signals-complex`](https://github.com/rust-dd/tako/tree/main/examples/signals-complex)
  — wildcard subscriptions and exporters.
