# 🐙 Tako — Lightweight Async Web Framework in Rust

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible async web framework for Rust.
> It aims to keep the mental model small while giving you first‑class performance and modern conveniences out‑of‑the‑box.

> **⚠️ Early‑stage software:** Tako is still under active development; use with caution and expect breaking changes.

> [**Blog**: Tako in detail](https://rust-dd.com/post/tako-a-lightweight-async-web-framework-on-tokio-and-hyper)


## ✨ Highlights

* **Batteries‑included Router** — Intuitive path‑based routing with path parameters and trailing‑slash redirection (TSR).
* **Extractor system** — Strongly‑typed request extractors for headers, query/body params, JSON, form data, etc.
* **Streaming & SSE** — Built‑in helpers for Server‑Sent Events *and* arbitrary `Stream` responses.
* **Middleware** — Compose synchronous or async middleware functions with minimal boilerplate.
* **Shared State** — Application‑wide state injection without `unsafe` globals.
* **Plugin system** — Opt‑in extensions let you add functionality without cluttering the core API.
* **Hyper‑powered** — Built on `hyper` & `tokio` for minimal overhead and async performance with **native HTTP/2 & TLS** support.

## Documentation

[API Documentation](https://docs.rs/tako-rs/latest/tako/) is mostly generated using AI from the source code. Please note that the documentation is still a work in progress and may contain errors or inaccuracies.

MSRV 1.87.0

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

    println!("Server running at http://127.0.0.1:8080");

    // Launch the server
    tako::serve(listener, router).await;

    Ok(())
}
```

## 📜 License

`MIT` — see [LICENSE](./LICENSE) for details.


Made with ❤️ & 🦀 by the Tako contributors.
