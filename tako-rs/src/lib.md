Tako is a multi-transport Rust web framework: one router, middleware stack, and
observability model for HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP,
UDP, and Unix sockets, on Tokio or Compio.

The package on crates.io is `tako-rs`; the library it provides is `tako`. The
separate `tako` crate on crates.io is an unrelated project.

# Quickstart

```toml
[dependencies]
tako-rs = "2.2"
tokio = { version = "1", features = ["macros", "net", "rt-multi-thread"] }
```

```rust,no_run
# #[cfg(not(feature = "compio"))]
# mod quickstart {
use tako::{router::Router, types::BoxError, Server};
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    let mut router = Router::new();
    router.get("/", || async { "Hello, Tako!" });

    let listener = TcpListener::bind("127.0.0.1:8080").await?;
    Server::builder()
        .build()
        .try_spawn_http(listener, router)?
        .result()
        .await?;

    Ok(())
}
# }
# fn main() {}
```

# Handlers and extractors

Handlers are async functions. Their arguments are extractors that pull typed
data out of the request, and their return values implement
[`Responder`](responder::Responder). Only the last argument may consume the
body.

```rust
use serde::{Deserialize, Serialize};
use tako::extractors::{json::Json, path::Path, state::State};
use tako::{responder::Responder, router::Router, StatusCode};

#[derive(Clone)]
struct AppState {
    greeting: String,
}

#[derive(Deserialize)]
struct NewUser {
    name: String,
}

#[derive(Serialize)]
struct User {
    id: u64,
    name: String,
}

async fn show_user(Path(id): Path<u64>, State(state): State<AppState>) -> String {
    format!("{}, user {id}", state.greeting)
}

async fn create_user(Json(new): Json<NewUser>) -> impl Responder {
    (StatusCode::CREATED, Json(User { id: 1, name: new.name }))
}

let mut router = Router::new();
router.with_state(AppState { greeting: "Hello".into() });
router.get("/users/{id}", show_user);
router.post("/users", create_user);
```

# Feature flags

The default build serves HTTP/1.1, raw TCP, and Unix sockets on Tokio, with the
core extractors and middleware. Everything else is opt-in:

| Feature | Enables |
| --- | --- |
| `http2`, `http3`, `tls` | HTTP/2 including h2c, HTTP/3 over QUIC, and rustls TLS |
| `ws`, `sse`, `udp` | WebSocket upgrades, Server-Sent Events, and UDP servers |
| `grpc`, `protobuf` | gRPC over HTTP/2 and protobuf bodies |
| `plugins` | CORS, compression, rate limiting, and idempotency |
| `signals`, `queue-cron` | In-process signals and cron-scheduled queue jobs |
| `metrics-prometheus`, `metrics-opentelemetry` | Metrics export |
| `async-graphql`, `utoipa`, `vespera` | GraphQL and `OpenAPI` integrations |
| `per-thread` | Thread-per-core server |
| `compio` | The Compio runtime: `io_uring` on Linux, IOCP on Windows |

The [feature reference](https://tako.rust-dd.com/docs/reference/features) lists
every flag.

# Learn more

- [Guide](https://tako.rust-dd.com/docs), also as
  [llms.txt](https://tako.rust-dd.com/llms.txt) for AI assistants
- [Runnable examples](https://github.com/rust-dd/tako/tree/main/examples)
- [Coming from Axum](https://tako.rust-dd.com/docs/getting-started/coming-from-axum)

# Crate layout

This umbrella crate stitches together the workspace sub-crates:

- `tako-rs-core`: routing, handlers, middleware and plugin traits, body and
  request types, state, signals, queue, plus GraphQL, gRPC, and `OpenAPI` helpers
- `tako-rs-extractors`: concrete request extractors (cookies, form, query,
  path, JWT, multipart, SIMD JSON, …)
- `tako-rs-server`: HTTP/1, TLS, HTTP/3, raw TCP, UDP, and Unix sockets, PROXY
  protocol, plus the Compio variants
- `tako-rs-streams`: WebSocket, SSE, file streaming, static file serving, and
  WebTransport
- `tako-rs-plugins`: built-in middleware (auth, CSRF, sessions, …) and plugins
  (CORS, compression, rate limiting, idempotency, metrics)

Public APIs are re-exported under `tako::*` according to the selected features.
