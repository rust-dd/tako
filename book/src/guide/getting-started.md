# Getting started

This chapter walks through installing Tako, writing your first handler,
and running it on a local listener. It mirrors the
[`examples/hello-world`](https://github.com/rust-dd/tako/tree/main/examples/hello-world)
crate in the repository.

## Install

Add Tako to `Cargo.toml`:

```toml
[dependencies]
tako-rs = "2"
tokio = { version = "1", features = ["full"] }
anyhow = "1"
```

The umbrella crate is published as `tako-rs`; everything is re-exported
under the `tako::*` path. You will see both spellings in the
ecosystem — `tako-rs` is the *package name*, `tako` is the *crate
name*.

The default feature set covers HTTP/1.1, WebSocket, SSE, raw TCP/UDP,
Unix sockets, and PROXY protocol on Tokio. Opt-in features such as
`tls`, `http2`, `http3`, `compio`, `multipart`, `simd`,
`metrics-prometheus`, and so on are listed in
[Cargo feature graph](../reference/features.md).

## Hello world

```rust
use anyhow::Result;
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;
use tokio::net::TcpListener;

async fn hello_world() -> impl Responder {
  "Hello, World!".into_response()
}

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", hello_world);

  tako::serve(listener, router).await;

  Ok(())
}
```

A few things to notice:

- A handler is any `async fn` whose arguments implement `FromRequest`
  / `FromRequestParts` and whose return type implements
  [`Responder`](https://docs.rs/tako-rs/latest/tako/responder/trait.Responder.html).
  In the snippet above the handler takes zero arguments, so it does
  not need to extract anything from the request.
- `Router::new()` produces an empty router. `route(Method::GET, ...,
  handler)` registers a handler against a `(method, path)` pair.
  Convenience shorthands (`router.get(path, handler)`, `.post`,
  `.put`, `.patch`, `.delete`, `.head`, `.options`) exist too.
- `tako::serve(listener, router)` is the simplest server entry point:
  it builds a default [`Server`](https://docs.rs/tako-rs/latest/tako/struct.Server.html)
  and drives it until the listener stops accepting. For finer control
  (graceful shutdown, drain timeouts, HTTP/2, TLS, etc.) use
  `Server::builder()` directly — see
  [Transports overview](./transports.md).

## Run it

```bash
cargo run
```

Then in another terminal:

```bash
curl http://127.0.0.1:8080/
# Hello, World!
```

## Adding a route with extractors

Handlers compose by adding more arguments. Tako will run the extractor
for each argument before invoking the handler:

```rust,ignore
use serde::Deserialize;
use tako::Method;
use tako::extractors::json::Json;
use tako::extractors::path::Path;
use tako::responder::Responder;
use tako::router::Router;

#[derive(Deserialize)]
struct UserPath { id: u64 }

#[derive(Deserialize)]
struct CreateUser { name: String }

async fn get_user(Path(p): Path<UserPath>) -> impl Responder {
  format!("user_id={}", p.id)
}

async fn create_user(Json(body): Json<CreateUser>) -> impl Responder {
  format!("created: {}", body.name)
}

let mut router = Router::new();
router.route(Method::GET, "/users/{id}", get_user);
router.route(Method::POST, "/users", create_user);
```

`Path<T>`, `Query<T>`, `Json<T>`, `Form<T>`, `Bytes`, `HeaderMap`,
`State<T>`, and the cookie / JWT / auth extractors all live under
`tako::extractors`. See the [Extractors](./extractors.md) chapter for
the full list.

## Return types

Anything that implements `Responder` can be returned from a handler.
The blanket impls cover the common cases:

- `&'static str` and `String` — `text/plain` body.
- `Bytes` / `Vec<u8>` — raw body.
- `Json<T>` — JSON body with `Content-Type: application/json`.
- `(StatusCode, T)` — status + body.
- `(StatusCode, HeaderMap, T)` — status + headers + body.
- `StatusCode` alone — empty body, just the status line.
- `Result<T, E>` where both arms implement `Responder` — the
  preferred shape for fallible handlers.

```rust
use http::StatusCode;
use tako::extractors::json::Json;
use serde::Serialize;

#[derive(Serialize)]
struct ApiError { message: &'static str }

async fn handler() -> Result<Json<&'static str>, (StatusCode, Json<ApiError>)> {
  Ok(Json("ok"))
}
```

## Next steps

- [Transports overview](./transports.md) — HTTP/2, HTTP/3, TLS,
  WebSocket, SSE, TCP/UDP, Unix sockets, PROXY protocol.
- [Routing](./routing.md) — nesting, scopes, path parameters,
  typed slots.
- [State](./state.md) — sharing configuration and dependencies with
  handlers.
- [Middleware](./middleware.md) — auth, CORS, compression, metrics,
  rate limiting.
