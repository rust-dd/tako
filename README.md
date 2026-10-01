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
- **Your choice of runtime** — Tokio or Compio; every transport runs on both.
- **Room to tune** — opt into SIMD JSON, zero-copy extractors, compression, or jemalloc when your workload calls for them.

## Quick start

Requires **Rust 1.95+**. Tako uses edition 2024.

Add these dependencies to your `Cargo.toml`:

```toml
[dependencies]
tako-rs = "2.4"
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

Upgrading? 2.4 needs no code changes; its
[migration guide](https://tako.rust-dd.com/docs/reference/migration-2-4) covers
the core pinning default, the header deadline, and extractor entries. The
[2.3 migration guide](https://tako.rust-dd.com/docs/reference/migration-2-3)
covers the removed HTTP client, WebTransport, and gRPC changes, the
[2.2 migration guide](https://tako.rust-dd.com/docs/reference/migration-2-2) the per-thread
changes, and the [2.1 migration guide](https://tako.rust-dd.com/docs/reference/migration-2-1)
the API changes and opt-in transport features from 2.0.

## Benchmarks

<picture>
  <source media="(prefers-color-scheme: dark)" srcset="https://raw.githubusercontent.com/rust-dd/tako/main/website/public/benchmarks/hello-world-dark.svg">
  <img alt="Hello-world requests per second under loopback, server-bound, and pipelined load for Tako per-thread with and without core pinning, Actix Web, ntex, Tako, Tako with jemalloc, and Axum" src="https://raw.githubusercontent.com/rust-dd/tako/main/website/public/benchmarks/hello-world-light.svg">
</picture>

| Framework | Loopback · 1,000 conns | Server-bound · 256 conns | Pipelined ×16 · 256 conns |
| --- | ---: | ---: | ---: |
| **Tako per-thread** | 1,842,063 | 612,091 | 15,033,668 |
| **Tako per-thread, pinned** | 1,780,810 | 611,956 | 15,716,822 |
| Actix Web | 1,786,640 | 570,865 | 12,806,477 |
| ntex | 1,740,750 | 543,076 | 8,749,294 |
| **Tako** | 1,401,625 | 483,526 | 11,512,467 |
| **Tako + `jemalloc`** | 1,386,954 | 469,807 | 10,853,271 |
| Axum | 1,132,259 | 389,844 | — |

Requests per second for `GET /` → `Hello, World!`. The thread-per-core
[`per-thread`](https://tako.rust-dd.com/docs/deployment#thread-per-core) server
serves 3% more requests than Actix Web on loopback, where kernel time
dominates, 7% more when the server is the bottleneck, and 17% more when `wrk`
pipelines 16 requests at a time. The pinned row turns on `pin_to_core`, which
is off by default. Tako's default server serves about 24% more than Axum on the
same Tokio runtime, at 1,000 connections and server-bound alike. Axum has no
pipelined number because `axum::serve` leaves `TCP_NODELAY` off.

Each number is the median of five 15-second `wrk` runs in a 24 vCPU Linux
container (AMD EPYC 9655P), measured with tako-rs 2.4.0 in October 2026.
Results move with hardware and configuration, so read the
[methodology and reproduction steps](https://tako.rust-dd.com/docs/benchmarks)
before drawing conclusions.

## License

`MIT` — see [LICENSE](./LICENSE).

Made with ❤️ & 🦀 by the Tako contributors.
