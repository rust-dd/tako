![Build Workflow](https://github.com/rust-dd/tako/actions/workflows/ci.yml/badge.svg)
[![Crates.io](https://img.shields.io/crates/v/tako-rs?style=flat-square)](https://crates.io/crates/tako-rs)
![License](https://img.shields.io/crates/l/tako-rs?style=flat-square)

# 🐙 Tako — Multi-Transport Rust Framework for Modern Network Services

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible Rust framework for services that go beyond plain HTTP.
> Build one cohesive application across HTTP/1.1, HTTP/2, HTTP/3, WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport with a single routing, middleware, and observability model.

> **Blog posts:**
> - [Tako: A Lightweight Async Web Framework on Tokio and Hyper](https://rust-dd.com/post/tako-a-lightweight-async-web-framework-on-tokio-and-hyper)
> - [Tako v.0.5.0 road to v.1.0.0](https://rust-dd.com/post/tako-v-0-5-0-road-to-v-1-0-0)
> - [Tako v0.5.0 → v0.7.1-2: from "nice router" to "mini platform"](https://rust-dd.com/post/tako-v0-5-0-to-v0-7-1-2-from-nice-router-to-mini-platform)

## Why Tako

Tako is built for teams that want fewer moving parts in production:

* **One service, many transports** — Serve REST, WebSockets, SSE, gRPC, raw TCP/UDP, Unix sockets, and QUIC-based workloads without switching frameworks.
* **One mental model, two runtimes** — Use the same framework style on **Tokio** or **Compio** depending on the deployment constraints.
* **Application primitives included** — Middleware, auth, metrics, signals, queues, graceful shutdown, and streaming are part of the framework story, not an afterthought.
* **Performance knobs when they matter** — SIMD JSON, optional zero-copy extractors, compression, jemalloc, and HTTP/3 support are available without fragmenting the API.
* **Strong fit for real systems** — API backends, realtime apps, protocol gateways, internal platforms, and edge-facing services.

## ✨ Highlights

* **Multi-transport by design** — HTTP/1.1, HTTP/2, HTTP/3 (QUIC), WebSocket, WebTransport, SSE, gRPC, TCP, UDP, Unix sockets, and PROXY protocol.
* **Dual runtime support** — First-class support for both **Tokio** and **Compio**, including TLS and HTTP/2 on both sides where supported.
* **Built-in platform primitives** — Background job queue, in-process signals, metrics hooks, graceful shutdown, static files, and stream helpers.
* **Rich middleware and auth** — JWT, Basic, Bearer, API key, CSRF, sessions, body limits, request IDs, security headers, upload progress, rate limiting, CORS, idempotency, and compression.
* **Strongly typed extraction** — 22+ extractors for JSON, form, query, path, headers, cookies, JWT claims, API keys, Accept, Range, protobuf, multipart, and more.
* **Performance paths included** — SIMD JSON (`sonic-rs` / `simd-json`), optional zero-copy extractors, brotli/gzip/deflate/zstd, and jemalloc support.
* **Realtime-ready** — Streaming responses, SSE, WebSockets, GraphQL subscriptions, HTTP/3, and WebTransport under one crate.
* **Docs and API surface included** — OpenAPI via `utoipa` or `vespera`, GraphiQL support, and a growing example suite for common deployment patterns.

## Best Fit

Choose Tako when your service needs one or more of these:

* **More than REST** — You need HTTP APIs plus WebSockets, SSE, gRPC, TCP, UDP, or QUIC in the same application.
* **Realtime coordination** — You want built-in signals, queues, and streaming primitives instead of composing everything manually.
* **Framework consolidation** — You would rather depend on one coherent crate than glue together several partially overlapping libraries.
* **Protocol-heavy infrastructure** — Gateways, internal platforms, telemetry collectors, control planes, or edge services are a particularly strong fit.

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

## Baseline Hello World Benchmark

Hello world throughput is not the whole story, but Tako is competitive even in the most reductionist comparison:

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
tako-rs = "1"
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
