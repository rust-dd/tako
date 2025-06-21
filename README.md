# 🐙 Tako — Lightweight Async Web Framework in Rust

> **Tako** (*"octopus"* in Japanese) is a pragmatic, ergonomic and extensible async web framework for Rust.
> It aims to keep the mental model small while giving you first‑class performance and modern conveniences out‑of‑the‑box.

---

## ✨ Highlights

| Feature                       | Description                                                                                                                                                          |
| ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Batteries‑included Router** | Intuitive path‑based routing with path parameters and trailing‑slash redirection (TSR).                                                                              |
| **Extractor system**          | Strongly‑typed request extractors for headers, query/body params, JSON, form data, etc.                                                                              |
| **Streaming & SSE**           | Built‑in helpers for Server‑Sent Events *and* arbitrary `Stream` responses.                                                                                          |
| **Middleware**                | Compose synchronous or async middleware functions with minimal boilerplate.                                                                                          |
| **Shared State**              | Application‑wide state injection without `unsafe` globals.                                                                                                           |
| **Hyper‑powered**             | Built on `hyper` & `tokio` for minimal overhead and async performance.<br><sub>HTTP/2 and native TLS integration are **WIP**</sub> |

---

## 📦 Installation

Add Tako to your **Cargo.toml** (the crate isn’t on crates.io yet, so pull it from Git):

```toml
[dependencies]
tako = { git = "https://github.com/rust-dd/tako", branch = "main" }
# tako = { path = "../tako" } # ← for workspace development
```

---

## 🚀 Quick Start

Below is a *minimal‑but‑mighty* example that demonstrates:

* Basic GET & POST routes with parameters
* Route‑scoped middleware
* Shared application state
* Server‑Sent Events (string & raw bytes streams)

```rust
use std::time::Duration;

use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use hyper::Method;
use serde::Deserialize;
use tako::{
    body::TakoBody,
    extractors::{bytes::Bytes as BodyBytes, header_map::HeaderMap, params::Params, FromRequest},
    responder::Responder,
    sse::{SseBytes, SseString},
    state::get_state,
    types::{Request, Response},
    ws::TakoWs,
};
use tokio_stream::{wrappers::IntervalStream, StreamExt};
use tokio_tungstenite::tungstenite::{Message, Utf8Bytes};

/// Global application state shared via an *arc‑swap* under the hood.
#[derive(Clone, Default)]
struct AppState {
    request_count: std::sync::atomic::AtomicU64,
}

/// `GET /` handler that echoes the body & headers back.
async fn hello(mut req: Request) -> impl Responder {
    let HeaderMap(headers) = HeaderMap::from_request(&mut req).await.unwrap();
    let BodyBytes(body) = BodyBytes::from_request(&mut req).await.unwrap();

    format!(
        "Hello, World!\n\nHeaders: {:#?}\nBody: {:?}",
        headers, body
    )
    .into_response()
}

/// Typed URL parameter struct for `/user/{id}`.
#[derive(Deserialize)]
struct UserParams {
    id: u32,
}

async fn create_user(mut req: Request) -> impl Responder {
    let Params(user) = Params::<UserParams>::from_request(&mut req).await.unwrap();
    let state = get_state::<AppState>("app_state").unwrap();
    state.request_count.fetch_add(1, std::sync::atomic::Ordering::Relaxed);

    format!("User {} created ✅", user.id).into_response()
}

/// String‑based SSE endpoint emitting `Hello` every second.
async fn sse_string(_: Request) -> impl Responder {
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
        .map(|_| "Hello".to_string());
    SseString { stream }
}

/// Raw‑bytes SSE variant (hand‑crafted frame).
async fn sse_bytes(_: Request) -> impl Responder {
    let stream = IntervalStream::new(tokio::time::interval(Duration::from_secs(1)))
        .map(|_| Bytes::from("data: hello\n\n"));
    SseBytes { stream }
}

/// Example auth middleware that short‑circuits with 401 when a header is missing.
async fn auth_middleware(req: Request) -> Result<Request, Response> {
    if req.headers().get("x-auth").is_none() {
        return Err(
            hyper::Response::builder()
                .status(401)
                .body(TakoBody::empty())
                .unwrap()
                .into_response(),
        );
    }
    Ok(req)
}

pub async fn ws_echo(req: Request) -> impl Responder {
    TakoWs::new(req, |mut ws| async move {
        let _ = ws.send(Message::Text("Welcome to Tako WS!".into())).await;

        while let Some(Ok(msg)) = ws.next().await {
            match msg {
                Message::Text(txt) => {
                    let _ = ws
                        .send(Message::Text(Utf8Bytes::from(format!("Echo: {txt}"))))
                        .await;
                }
                Message::Binary(bin) => {
                    let _ = ws.send(Message::Binary(bin)).await;
                }
                Message::Ping(p) => {
                    let _ = ws.send(Message::Pong(p)).await;
                }
                Message::Close(_) => {
                    let _ = ws.send(Message::Close(None)).await;
                    break;
                }
                _ => {}
            }
        }
    })
}

pub async fn ws_tick(req: Request) -> impl Responder {
    TakoWs::new(req, |mut ws| async move {
        let mut ticker =
            IntervalStream::new(tokio::time::interval(Duration::from_secs(1))).enumerate();

        loop {
            tokio::select! {
                msg = ws.next() => {
                    match msg {
                        Some(Ok(Message::Close(_))) | None => break,
                        _ => {}
                    }
                }

                Some((i, _)) = ticker.next() => {
                    let _ = ws.send(Message::Text(Utf8Bytes::from(format!("tick #{i}")))).await;
                }
            }
        }
    })
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;

    let mut router = tako::router::Router::new();
    router.state("app_state", AppState::default());

    // Routes --------------------------------------------------------------
    router
        .route(Method::GET, "/", hello)
        .middleware(auth_middleware);

    router.route_with_tsr(Method::POST, "/user/{id}", create_user);
    router.route_with_tsr(Method::GET, "/sse/string", sse_string);
    router.route_with_tsr(Method::GET, "/sse/bytes", sse_bytes);
    router.route_with_tsr(Method::GET, "/ws/echo", ws_echo);
    router.route_with_tsr(Method::GET, "/ws/tick", ws_tick);

    // Start the server (HTTP/1.1 — HTTP/2 coming soon!)
    #[cfg(not(feature = "tls"))]
    tako::serve(listener, r).await;

    #[cfg(feature = "tls")]
    tako::serve_tls(listener, r).await;

    Ok(())
}
```

> **Tip:** Tako returns a **308 Permanent Redirect** automatically when the trailing slash in the request does not match your route declaration. Use `route_with_tsr` when you *want* that redirect.

---

## 🧑‍💻 Development & Contributing

1. **Clone** the repo and run the examples:

   ```bash
   git clone https://github.com/rust-dd/tako
   cd tako && cargo run --example hello_world
   ```
2. Format & lint:

   ```bash
   cargo fmt && cargo clippy --all-targets --all-features
   ```
3. Open a PR – all contributions, big or small, are welcome!

---

## 🧪 Running the Example Above

```bash
cargo run  # in the folder with `main.rs`
```

Navigate to [http://localhost:8080/](http://localhost:8080/) and watch requests stream in your terminal.

For the SSE endpoints:

```bash
curl -N http://localhost:8080/sse/string   # string frames
curl -N http://localhost:8080/sse/bytes    # raw bytes
```

---

## 📜 License

MIT

---

Made with ❤️ & 🦀 by the Tako contributors.
