![Build Workflow](https://github.com/rust-dd/tako/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/tako-rs?style=flat-square)](https://crates.io/crates/tako-rs)
![License](https://img.shields.io/crates/l/tako-rs?style=flat-square)

# 🐙 Tako — Multi-Transport Rust Framework for Modern Network Services

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible Rust framework for services that go beyond plain HTTP.
> Build one cohesive application across HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport with a single routing, middleware, and observability model.

📖 **Full documentation → [tako.rust-dd.com](https://tako.rust-dd.com)** &nbsp;·&nbsp; [API docs (docs.rs)](https://docs.rs/tako-rs/latest/tako/) &nbsp;·&nbsp; [Release notes](https://github.com/rust-dd/tako/releases)

## Why Tako

- **Typed handlers** — ordinary async functions, typed request extractors, and flexible response types.
- **Beyond HTTP** — add WebSockets, event streams, gRPC, or raw socket services as your application grows.
- **Your choice of runtime** — Tokio or Compio, with TLS and HTTP/2 support on both.
- **Room to tune** — opt into SIMD JSON, zero-copy extractors, compression, or jemalloc when your workload calls for them.

## Quick start

Requires **Rust 1.95+**. Tako uses edition 2024.

Add these dependencies to your `Cargo.toml`:

```toml
[dependencies]
tako-rs = "2.1"
tokio = { version = "1", features = ["macros", "net", "rt-multi-thread"] }
```

Then create `src/main.rs`. The package is `tako-rs`; the Rust import is `tako`.

```rust
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
```

Start it with `cargo run`, then visit <http://127.0.0.1:8080> or run:

```sh
curl http://127.0.0.1:8080/
```

Continue with the [Quickstart guide](https://tako.rust-dd.com/docs/getting-started/quickstart)
or explore the [runnable examples](./examples).

## At a glance

| Area | Capabilities |
| --- | --- |
| Transports | HTTP/1.1, HTTP/2, HTTP/3, WebSocket, WebTransport, SSE, TCP, UDP, Unix sockets, PROXY protocol |
| Extractors | JSON, form, query, path, headers, cookies, JWT claims, API keys, multipart, protobuf |
| Middleware | Authentication, CSRF, sessions, security headers, request IDs, body limits, rate limiting, CORS, idempotency, compression |
| Integrations | GraphQL, unary gRPC, OpenAPI, Prometheus, OpenTelemetry, queues, signals |

The default setup uses Tokio and includes HTTP/1.1. Enable additional protocols
and integrations through [Cargo features](https://tako.rust-dd.com/docs/reference/features).
See the [runtime compatibility guide](https://tako.rust-dd.com/docs/concepts/runtimes)
for transport support on Tokio and Compio.

For upgrades from 2.0, the [2.1 migration guide](https://tako.rust-dd.com/docs/reference/migration-2-1)
covers API changes and the new opt-in transport features.

## Benchmarks

Hello-world throughput on a clean local run (`wrk -t4 -c100 -d30s`):

| Framework | Requests/sec | Avg Latency |
| --- | ---: | ---: |
| Tako | ~187,288 | ~505 µs |
| Tako + `jemalloc` | ~187,638 | ~502 µs |
| Axum | ~186,194 | ~498 µs |
| Actix | ~155,307 | ~635 µs |

Results depend on hardware, configuration, and thermal state. See the
[benchmark setup and results](https://tako.rust-dd.com/docs/benchmarks) for context.

## License

`MIT` — see [LICENSE](./LICENSE).

Made with ❤️ & 🦀 by the Tako contributors.
