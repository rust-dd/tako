# State

Tako gives handlers access to shared application state — database
pools, configuration, signal arbiters, queue handles — through two
mechanisms:

- **Per-router state** via `Router::with_state(value)`. New in 2.0.
  Each `Router` carries its own typed store, so two routers in the
  same process can hold different values of the same type.
- **Process-global state** via `tako::state::{set_state, get_state}`.
  Inherited from 1.x; still supported. The store is keyed by
  `TypeId`, so there is only one slot per `T` per process.

The `State<T>` extractor reads from the per-router store first and
falls back to the process-global store, so most code only needs to
care about one of the two paths at a time.

## Per-router state (recommended)

```rust
use std::sync::Arc;
use tako::Method;
use tako::extractors::state::State;
use tako::responder::Responder;
use tako::router::Router;

#[derive(Clone)]
struct AppConfig {
  greeting: &'static str,
}

async fn hello(State(cfg): State<AppConfig>) -> impl Responder {
  format!("{}, world!", cfg.greeting)
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.with_state(AppConfig { greeting: "Hello" });
  router.route(Method::GET, "/", hello);

  tako::serve(listener, router).await;
  Ok(())
}
```

`with_state` writes into the router's `Arc<RouterState>`. The
`State<T>` extractor surfaces the stored value as `Arc<T>`. The hot
path is fast-checked with an `AtomicBool::Acquire`: when no caller
ever invoked `with_state`, the state lookup short-circuits and adds
no measurable overhead.

You can call `with_state` multiple times, once per type:

```rust,ignore
let mut router = Router::new();
router
  .with_state(db_pool)
  .with_state(redis_pool)
  .with_state(metrics);
```

Each handler can extract any subset:

```rust,ignore
async fn handler(
  State(db): State<DbPool>,
  State(metrics): State<MetricsHandle>,
) -> impl Responder { ... }
```

## Multiple routers, distinct state

A single process can host several routers — for example a public API
on port `8080` and an internal admin API on `8081` — each with its
own state:

```rust,ignore
let mut public = Router::new();
public.with_state(public_cfg);
public.get("/", hello);

let mut admin = Router::new();
admin.with_state(admin_cfg);
admin.get("/metrics", scrape_metrics);

let server = Server::builder().build();
let public_h = server.spawn_http(public_listener, public);
let admin_h  = server.spawn_http(admin_listener, admin);

tokio::join!(public_h.join(), admin_h.join());
```

Both routers carry an `AppConfig`, but the values differ. With
`GLOBAL_STATE` this required a newtype wrapper per router; with
`with_state` it just works.

## Process-global state (legacy)

The 1.x pattern is still available — useful when you don't have a
`Router` in hand (background tasks, signal handlers, queue workers):

```rust,ignore
use tako::state::{get_state, set_state};

#[derive(Clone)]
struct Counter(u64);

set_state(Counter(0));

// later, from a background task:
if let Some(counter) = get_state::<Counter>() {
  println!("count = {}", counter.0);
}
```

`set_state` registers a value keyed by its type. `get_state::<T>()`
returns `Option<Arc<T>>`. Because the store is global and keyed by
`TypeId`, two callers cannot store distinct `T` values for the same
`T` — last writer wins. Reach for `with_state` whenever the value
belongs to a single router.

See [`examples/with-state`](https://github.com/rust-dd/tako/tree/main/examples/with-state)
for a runnable demonstration, and the
[Migration guide](../reference/migration.md) for the full upgrade
path from `GLOBAL_STATE` to `Router::with_state`.
