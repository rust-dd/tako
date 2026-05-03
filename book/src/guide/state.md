# State

> **Status:** scaffold.

```rust,ignore
let router = Router::new()
    .with_state(Config { db, cfg });

async fn handler(State(cfg): State<Config>) -> impl Responder { ... }
```

`Router::with_state(value)` writes into a per-router store. The
`State<T>` extractor reads from the per-router store first and falls
back to `GLOBAL_STATE` for backward compatibility. Two routers in the
same process can hold different `T` values without newtype wrappers.

The hot path is fast-checked with an `AtomicBool::Acquire` — when no
caller invoked `with_state`, the state lookup short-circuits and adds
no measurable overhead.
