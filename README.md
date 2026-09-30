![Build Workflow](https://github.com/rust-dd/tako/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/tako-rs?style=flat-square)](https://crates.io/crates/tako-rs)
![License](https://img.shields.io/crates/l/tako-rs?style=flat-square)

# 🐙 Tako — Multi-Transport Rust Framework for Modern Network Services

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible Rust framework for services that go beyond plain HTTP.
> Build one cohesive application across HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport with a single routing, middleware, and observability model.

📖 **Full documentation → [tako.rust-dd.com](https://tako.rust-dd.com)** &nbsp;·&nbsp; [API docs (docs.rs)](https://docs.rs/tako-rs/latest/tako/) &nbsp;·&nbsp; [Release notes](https://github.com/rust-dd/tako/releases) &nbsp;·&nbsp; [llms.txt](https://tako.rust-dd.com/llms.txt) for AI assistants

## Why Tako

- **Typed handlers** — ordinary async functions, typed request extractors, and flexible response types.
- **Beyond HTTP** — add WebSockets, event streams, gRPC, or raw socket services as your application grows.
- **Your choice of runtime** — Tokio or Compio; every transport except WebTransport runs on both.
- **Room to tune** — opt into SIMD JSON, zero-copy extractors, compression, or jemalloc when your workload calls for them.

## Quick start

Requires **Rust 1.95+**. Tako uses edition 2024.

Add these dependencies to your `Cargo.toml`:

```toml
[dependencies]
tako-rs = "2.2"
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
or explore the [runnable examples](./examples). Coming from Axum? The
[Axum guide](https://tako.rust-dd.com/docs/getting-started/coming-from-axum) maps
each building block, and the [comparison](https://tako.rust-dd.com/docs/concepts/comparison)
covers when to pick Tako, Axum, or Actix Web.

## At a glance

| Area | Capabilities |
| --- | --- |
| Transports | HTTP/1.1, HTTP/2, HTTP/3, WebSocket, WebTransport, SSE, TCP, UDP, Unix sockets, PROXY protocol |
| Extractors | JSON, form, query, path, headers, cookies, JWT claims, API keys, multipart, protobuf |
| Middleware | Authentication, CSRF, sessions, security headers, request IDs, body limits, rate limiting, CORS, idempotency, compression |
| Integrations | GraphQL, gRPC (unary and streaming), OpenAPI, Prometheus, OpenTelemetry, queues, signals |

The default setup uses Tokio and includes HTTP/1.1. Enable additional protocols
and integrations through [Cargo features](https://tako.rust-dd.com/docs/reference/features).
See the [runtime compatibility guide](https://tako.rust-dd.com/docs/concepts/runtimes)
for transport support on Tokio and Compio.

Upgrading? The [2.2 migration guide](https://tako.rust-dd.com/docs/reference/migration-2-2)
covers the per-thread changes, and the [2.1 migration guide](https://tako.rust-dd.com/docs/reference/migration-2-1)
covers the API changes and opt-in transport features from 2.0.

## Benchmarks

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rust-dd/tako/main/website/public/benchmarks/hello-world-dark.svg">
  <img alt="Hello-world requests per second at 100 and 1,000 connections for Actix Web, Tako per-thread, ntex, Tako with jemalloc, Axum, and Tako" src="https://raw.githubusercontent.com/rust-dd/tako/main/website/public/benchmarks/hello-world-light.svg">
</picture>

| Framework | 100 conns · req/s | p99 | 1,000 conns · req/s | p99 |
| --- | ---: | ---: | ---: | ---: |
| Actix Web | 1,530,425 | 3.48 ms | 1,580,809 | 6.79 ms |
| ntex | 1,562,670 | 3.23 ms | 1,557,559 | 6.25 ms |
| **Tako per-thread** | 1,436,252 | 6.11 ms | 1,521,982 | 7.89 ms |
| **Tako + `jemalloc`** | 497,515 | 1.56 ms | 1,286,328 | 3.05 ms |
| **Tako** | 478,502 | 1.73 ms | 1,112,141 | 3.64 ms |
| Axum | 496,454 | 1.48 ms | 1,048,804 | 3.81 ms |

Tako's default server keeps pace with Axum on the same Tokio runtime and pulls
ahead at 1,000 connections, and the thread-per-core
[`per-thread`](https://tako.rust-dd.com/docs/deployment#thread-per-core) server
lands within a few percent of Actix Web and ntex. On the multi-threaded server,
installing jemalloc through the `jemalloc` feature adds about 16% at 1,000
connections. `GET /` returns `Hello, World!`; each
number is the median of three 20-second `wrk` runs over loopback in a 24 vCPU
Linux container (AMD EPYC 9655P), measured with tako-rs 2.2.0 in September 2026.
Results move with hardware and configuration, so read the
[methodology and reproduction steps](https://tako.rust-dd.com/docs/benchmarks)
before drawing conclusions.

## License

`MIT` — see [LICENSE](./LICENSE).

Made with ❤️ & 🦀 by the Tako contributors.
