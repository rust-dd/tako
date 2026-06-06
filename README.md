![Build Workflow](https://github.com/rust-dd/tako/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/tako-rs?style=flat-square)](https://crates.io/crates/tako-rs)
![License](https://img.shields.io/crates/l/tako-rs?style=flat-square)

# 🐙 Tako — Multi-Transport Rust Framework for Modern Network Services

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible Rust framework for services that go beyond plain HTTP.
> Build one cohesive application across HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport with a single routing, middleware, and observability model.

📖 **Full documentation → [tako.rust-dd.com](https://tako.rust-dd.com)** &nbsp;·&nbsp; [API docs (docs.rs)](https://docs.rs/tako-rs/latest/tako/)

## Why Tako

- **One service, many transports** — REST, WebSockets, SSE, gRPC, raw TCP/UDP, Unix sockets, and QUIC without switching frameworks.
- **One model, two runtimes** — the same framework style on **Tokio** or **Compio**, TLS and HTTP/2 on both.
- **Batteries included** — middleware, auth, metrics, signals, queues, graceful shutdown, and streaming are part of the framework, not an afterthought.
- **Performance when it matters** — SIMD JSON, optional zero-copy extractors, brotli/gzip/deflate/zstd, jemalloc, and HTTP/3 — without fragmenting the API.

## At a glance

- **Transports** — HTTP/1.1, HTTP/2, HTTP/3 (QUIC), WebSocket, WebTransport, SSE, gRPC, TCP, UDP, Unix sockets, PROXY protocol.
- **Extraction** — 22+ typed extractors: JSON (SIMD optional), form, query, path, headers, cookies, JWT claims, API keys, Accept, Range, protobuf, multipart.
- **Middleware** — JWT/Basic/Bearer/API-key auth, CSRF, sessions, security headers, request IDs, body limits, rate limiting, CORS, idempotency, compression, metrics.

The full transport matrix, extractor catalog, middleware reference, and cargo
feature graph live in the [documentation](https://tako.rust-dd.com).

## Installation

```toml
[dependencies]
tako-rs = "2"
```

MSRV 1.95 · Edition 2024

## Quick Start

```rust
use anyhow::Result;
use tako::{
    responder::Responder,
    router::Router,
    types::Request,
    Method,
};
use tokio::net::TcpListener;

async fn hello_world(_: Request) -> impl Responder {
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

Keep going with the [Quickstart guide](https://tako.rust-dd.com/docs/getting-started/quickstart).

## In Production

Tako already powers real-world services:

- `stochastic-api` — https://stochasticlab.cloud/
- `shrtn.ink` — https://app.shrtn.ink/

## Benchmark

Hello-world throughput on a clean local run (`wrk -t4 -c100 -d30s`):

| Framework | Requests/sec | Avg Latency |
| --- | ---: | ---: |
| Tako | ~187,288 | ~505 µs |
| Tako + `jemalloc` | ~187,638 | ~502 µs |
| Axum | ~186,194 | ~498 µs |
| Actix | ~155,307 | ~635 µs |

Machine- and thermal-state-dependent — treat as local baselines, not universal
claims. Details on the [benchmarks page](https://tako.rust-dd.com/docs/benchmarks).

## License

`MIT` — see [LICENSE](./LICENSE).

Made with ❤️ & 🦀 by the Tako contributors.
