![Build Workflow](https://github.com/rust-dd/tako/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/tako-rs?style=flat-square)](https://crates.io/crates/tako-rs)
![License](https://img.shields.io/crates/l/tako-rs?style=flat-square)

# 🐙 Tako — Lightweight Async Web Framework in Rust

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible async web framework for Rust.
> It aims to keep the mental model small while giving you first‑class performance and modern conveniences out‑of‑the‑box.

> **Blog posts:**
> - [Tako: A Lightweight Async Web Framework on Tokio and Hyper](https://rust-dd.com/post/tako-a-lightweight-async-web-framework-on-tokio-and-hyper)
> - [Tako v.0.5.0 road to v.1.0.0](https://rust-dd.com/post/tako-v-0-5-0-road-to-v-1-0-0)
> - [Tako v0.5.0 → v0.7.1-2: from "nice router" to "mini platform"](https://rust-dd.com/post/tako-v0-5-0-to-v0-7-1-2-from-nice-router-to-mini-platform)


## ✨ Highlights

* **Multi-protocol** — HTTP/1.1, HTTP/2, HTTP/3 (QUIC), WebSocket, WebTransport, SSE, gRPC, TCP, UDP, Unix sockets, PROXY protocol.
* **Dual runtime** — First-class support for both **Tokio** and **Compio** async runtimes (including TLS + HTTP/2 on both).
* **22+ extractors** — Strongly-typed request extractors: JSON, form, query, path, headers, cookies (signed/private), JWT, Basic/Bearer auth, API key, Accept, Range, protobuf, and more.
* **Rich middleware** — Auth (JWT, Basic, Bearer, API key), CSRF, sessions, body limits, request IDs, security headers, upload progress, compression, rate limiting, CORS, idempotency, metrics.
* **SIMD JSON** — Route-level configurable SIMD-accelerated JSON parsing (sonic-rs / simd-json) with automatic size-based dispatch.
* **Background job queue** — In-memory task queue with named handlers, retry policies (fixed / exponential backoff), delayed jobs, dead letter queue, and graceful shutdown.
* **Streaming & SSE** — Built-in helpers for Server-Sent Events and arbitrary `Stream` responses.
* **GraphQL** — Async-GraphQL integration with extractors, responses, and WebSocket subscriptions.
* **OpenAPI** — Automatic API documentation via utoipa or vespera integration.
* **TLS** — Native rustls-based HTTPS with ALPN negotiation.
* **Compression** — Brotli, gzip, deflate, and zstd response compression.
* **Signals** — In-process pub/sub signal system for decoupled event-driven architectures.
* **Static files & file streaming** — Serve directories and stream files with range request support.
* **Graceful shutdown** — Drain in-flight connections before exit across all server variants.

## Feature Matrix

### Transports & Protocols

| Protocol | Tokio | Compio | Feature flag |
|---|---|---|---|
| HTTP/1.1 | ✅ | ✅ | *default* |
| HTTP/2 | ✅ | ✅ | `http2` |
| HTTP/3 (QUIC) | ✅ | — | `http3` |
| TLS (rustls) | ✅ | ✅ | `tls` / `compio-tls` |
| WebSocket | ✅ | ✅ | *default* / `compio-ws` |
| WebTransport | ✅ | — | `webtransport` |
| SSE | ✅ | ✅ | *default* |
| gRPC (unary) | ✅ | — | `grpc` |
| Raw TCP | ✅ | — | *default* |
| Raw UDP | ✅ | — | *default* |
| Unix sockets | ✅ | — | *default* (unix only) |
| PROXY protocol v1/v2 | ✅ | — | *default* |

### Extractors (22+)

| Extractor | Description |
|---|---|
| `Json<T>` | JSON body (with optional SIMD acceleration) |
| `Form<T>` | URL-encoded form body |
| `Query<T>` | URL query parameters |
| `Path<T>` | Route path parameters |
| `Params` | Dynamic path params map |
| `HeaderMap` | Full request headers |
| `Bytes` | Raw request body |
| `State<T>` | Shared application state |
| `CookieJar` | Cookie reading/writing |
| `SignedCookieJar` | HMAC-signed cookies |
| `PrivateCookieJar` | Encrypted cookies |
| `BasicAuth` | HTTP Basic authentication |
| `BearerAuth` | Bearer token extraction |
| `JwtClaims<T>` | JWT token validation & claims |
| `ApiKey` | API key from header/query |
| `Accept` | Content negotiation |
| `AcceptLanguage` | Language negotiation |
| `Range` | HTTP Range header |
| `IpAddr` | Client IP address |
| `Protobuf<T>` | Protocol Buffers body |
| `SimdJson<T>` | Force SIMD JSON parsing |
| `Multipart` | Multipart form data |

### Middleware

| Middleware | Description |
|---|---|
| JWT Auth | Validate JWT tokens on routes |
| Basic Auth | HTTP Basic authentication |
| Bearer Auth | Bearer token validation |
| API Key Auth | Header or query-based API key |
| CSRF | Double-submit cookie CSRF protection |
| Session | Cookie-based sessions (in-memory store) |
| Security Headers | HSTS, X-Frame-Options, CSP, etc. |
| Request ID | Generate/propagate `X-Request-ID` |
| Body Limit | Enforce max request body size |
| Upload Progress | Track upload progress callbacks |
| CORS | Cross-Origin Resource Sharing |
| Rate Limiter | Token-bucket rate limiting |
| Compression | Brotli / gzip / deflate / zstd |
| Idempotency | Idempotency key deduplication |
| Metrics | Prometheus / OpenTelemetry export |

### Feature Flags

| Flag | Description |
|---|---|
| `http2` | HTTP/2 support (ALPN h2) |
| `http3` | HTTP/3 over QUIC (enables `webtransport`) |
| `tls` | HTTPS via rustls |
| `compio` | Compio async runtime (alternative to tokio) |
| `compio-tls` | TLS on compio |
| `compio-ws` | WebSocket on compio |
| `grpc` | gRPC unary RPCs with protobuf |
| `protobuf` | Protobuf extractor (prost) |
| `plugins` | CORS, compression, rate limiting |
| `simd` | SIMD JSON parsing (sonic-rs + simd-json) |
| `multipart` | Multipart form-data extractors |
| `file-stream` | File streaming & range requests |
| `async-graphql` | GraphQL integration |
| `graphiql` | GraphiQL IDE endpoint |
| `signals` | In-process pub/sub signal system |
| `jemalloc` | jemalloc global allocator |
| `zstd` | Zstandard compression (in plugins) |
| `tako-tracing` | Distributed tracing subscriber |
| `utoipa` | OpenAPI docs via utoipa |
| `vespera` | OpenAPI docs via vespera |
| `metrics-prometheus` | Prometheus metrics export |
| `metrics-opentelemetry` | OpenTelemetry metrics export |
| `zero-copy-extractors` | Zero-copy body extraction |
| `client` | Outbound HTTP client |

## Documentation

[API Documentation](https://docs.rs/tako-rs/latest/tako/)

MSRV 1.87.0 | Edition 2024

## Tako in Production

Tako already powers real-world services in production:

- `stochastic-api`: https://stochasticlab.cloud/
- `shrtn.ink`: https://app.shrtn.ink/

## 🔥 Benchmarking the Hello World

```
+---------------------------+------------------+------------------+---------------+
| Framework 🦀              |   Requests/sec   |   Avg Latency    | Transfer/sec  |
+---------------------------+------------------+------------------+---------------+
| Tako (not taco! 🌮)       |    ~148,800      |    ~649 µs       |   ~12.6 MB/s  |
| Tako Jemalloc             |    ~158,059      |    ~592 µs       |   ~13.3 MB/s  |
| Axum                      |    ~153,500      |    ~607 µs       |   ~19 MB/s    |
| Actix                     |    ~126,300      |    ~860 µs       |   ~15.7 MB/s  |
+---------------------------+------------------+------------------+---------------+

👉 Command used: `wrk -t4 -c100 -d30s http://127.0.0.1:8080/`
```


## 📦 Installation

Add **Tako** to your `Cargo.toml`:

```toml
[dependencies]
tako-rs = "*"
```


## 🚀 Quick Start

Spin up a "Hello, World!" server in a handful of lines:

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
    // Bind a local TCP listener
    let listener = TcpListener::bind("127.0.0.1:8080").await?;

    // Declare routes
    let mut router = Router::new();
    router.route(Method::GET, "/", hello_world);

    // Launch the server
    tako::serve(listener, router).await;

    Ok(())
}
```

## 📜 License

`MIT` — see [LICENSE](./LICENSE) for details.


Made with ❤️ & 🦀 by the Tako contributors.
