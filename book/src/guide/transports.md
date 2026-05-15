# Transports overview

Tako runs the same `Router` across multiple transports. The
[`Server`](https://docs.rs/tako-rs/latest/tako/struct.Server.html)
builder picks which transport(s) a given listener serves; every spawn
method returns a
[`ServerHandle`](https://docs.rs/tako-rs/latest/tako/struct.ServerHandle.html)
that you can `join().await` or drive into a graceful drain.

| Transport | Spawn method | Cargo feature |
|---|---|---|
| HTTP/1.1 | `Server::spawn_http` / `tako::serve` | *default* |
| HTTP/2 cleartext (h2c) | `Server::spawn_h2c` | `http2` |
| HTTP/1.1 + HTTP/2 over TLS (ALPN) | `Server::spawn_tls` | `tls` (+ `http2` for ALPN) |
| HTTP/3 (QUIC) | `Server::spawn_h3` / `tako::serve_h3` | `http3` |
| Unix domain socket (HTTP) | `Server::spawn_unix_http` | *default* (Unix) |
| vsock (HTTP) | `Server::spawn_vsock_http` | `vsock` (Linux) |
| Raw TCP | `Server::spawn_tcp_raw` / `tako::server_tcp::serve_tcp` | *default* |
| Raw UDP | `Server::spawn_udp_raw` / `tako::server_udp::serve_udp` | *default* |
| PROXY protocol (v1/v2) | `Server::spawn_proxy_protocol` | *default* (via `tako-server-pt` for the standalone parser) |
| WebSocket upgrade | `TakoWs` inside an HTTP handler | *default* |
| Server-Sent Events | `tako::sse::Sse` inside an HTTP handler | *default* |

A single `ServerConfig` flows into every transport, so header
read timeouts, keep-alive, drain timeout, max connections, h2 caps,
h3 caps, and PROXY read timeout are all sourced from one struct.

## HTTP/1.1 (default)

The simplest entry point — `tako::serve` builds a default server,
binds the listener, and accepts until the process exits:

```rust
use anyhow::Result;
use tako::Method;
use tako::router::Router;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;

  let mut router = Router::new();
  router.route(Method::GET, "/", || async { "hello" });

  tako::serve(listener, router).await;
  Ok(())
}
```

For full control over drain timeouts, max connections, or graceful
shutdown, build a `Server` explicitly:

```rust
use std::time::Duration;
use tako::{Server, ServerConfig};
use tako::router::Router;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8080").await?;
  let mut router = Router::new();
  router.get("/", || async { "hi" });

  let server = Server::builder()
    .config(ServerConfig {
      drain_timeout: Duration::from_secs(30),
      max_connections: Some(10_000),
      ..ServerConfig::default()
    })
    .build();

  let handle = server.spawn_http(listener, router);
  tokio::signal::ctrl_c().await?;
  handle.shutdown(Duration::from_secs(30)).await;
  Ok(())
}
```

See [`examples/hello-world`](https://github.com/rust-dd/tako/tree/main/examples/hello-world).

## HTTP/2 (cleartext + TLS / ALPN)

Enable the `http2` feature. For prior-knowledge h2c (typically used
behind L7 proxies like Envoy or Nginx that terminate TLS and forward
cleartext h2c upstream) call `spawn_h2c`. For browser-facing h2,
combine `http2` with `tls` and use `spawn_tls` — `Server` negotiates
HTTP/2 via ALPN automatically:

```rust
use tako::{Server, TlsCert};
use tako::router::Router;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let listener = TcpListener::bind("127.0.0.1:8443").await?;
  let mut router = Router::new();
  router.get("/", || async { "h2!" });

  let server = Server::builder()
    .tls(TlsCert::pem_paths("cert.pem", "key.pem"))
    .build();

  let handle = server.spawn_tls(listener, router);
  handle.join().await;
  Ok(())
}
```

H2 caps (max concurrent streams, initial window size, keep-alive
interval, etc.) live on `ServerConfig`.

## HTTP/3 (QUIC)

Behind the `http3` feature. HTTP/3 mandates TLS, so a `TlsCert` is
required on the builder. The address is passed at spawn time because
QUIC binds a UDP socket, not a `TcpListener`:

```rust
use tako::{Server, TlsCert};
use tako::router::Router;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let mut router = Router::new();
  router.get("/", || async { "Hello over HTTP/3" });

  let server = Server::builder()
    .tls(TlsCert::pem_paths("cert.pem", "key.pem"))
    .build();

  let handle = server.spawn_h3("[::]:4433", router);
  handle.join().await;
  Ok(())
}
```

For the simple "boot a single HTTP/3 server until ctrl-c" shape,
`tako::serve_h3(router, addr, Some("cert.pem"), Some("key.pem"))`
is available too. See
[`examples/hello-world-http3`](https://github.com/rust-dd/tako/tree/main/examples/hello-world-http3).

## Unix sockets

Useful for sidecar processes that talk to a local supervisor or for
running an admin endpoint without binding a TCP port. The same
`Router` is reused:

```rust
use tako::router::Router;
use tako::server_unix::serve_unix_http;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let mut router = Router::new();
  router.get("/", || async { "hello from unix" });

  serve_unix_http("/tmp/tako.sock", router).await;
  Ok(())
}
```

`Server::builder().build().spawn_unix_http(path, router)` is the
builder-style equivalent. See
[`examples/unix-socket`](https://github.com/rust-dd/tako/tree/main/examples/unix-socket).

## Raw TCP

`spawn_tcp_raw` (and the standalone `tako::server_tcp::serve_tcp`
helper) skips the HTTP layer entirely — the handler closure receives
each accepted stream so you can implement a custom line protocol,
relay, or proxy:

```rust
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tako::server_tcp::serve_tcp("127.0.0.1:9001", |mut stream, addr| {
    Box::pin(async move {
      let mut buf = vec![0u8; 4096];
      loop {
        let n = stream.read(&mut buf).await?;
        if n == 0 { break; }
        stream.write_all(&buf[..n]).await?;
        eprintln!("echoed {n} bytes for {addr}");
      }
      Ok(())
    })
  })
  .await?;
  Ok(())
}
```

See [`examples/tcp-echo`](https://github.com/rust-dd/tako/tree/main/examples/tcp-echo).

## Raw UDP

Datagram-oriented: the handler receives each packet plus a clone of
the socket for replying.

```rust
#[tokio::main]
async fn main() -> anyhow::Result<()> {
  tako::server_udp::serve_udp("127.0.0.1:9000", |data, addr, socket| {
    Box::pin(async move {
      let _ = socket.send_to(&data, addr).await;
    })
  })
  .await?;
  Ok(())
}
```

See [`examples/udp-echo`](https://github.com/rust-dd/tako/tree/main/examples/udp-echo).

## WebSocket

WebSocket lives inside the HTTP handler — `TakoWs::new(req, fut)`
performs the upgrade and hands you a `tokio_tungstenite` `WebSocket`
pair (or `compio_ws`'s equivalent on the compio runtime):

```rust
use futures_util::{SinkExt, StreamExt};
use tako::Method;
use tako::responder::Responder;
use tako::types::Request;
use tako::ws::TakoWs;
use tokio_tungstenite::tungstenite::Message;

async fn ws_echo(req: Request) -> impl Responder {
  TakoWs::new(req, |mut ws| async move {
    while let Some(Ok(msg)) = ws.next().await {
      if let Message::Text(t) = msg {
        let _ = ws.send(Message::Text(t)).await;
      }
    }
  })
}

#[tokio::main]
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await.unwrap();
  let mut router = tako::router::Router::new();
  router.route(Method::GET, "/ws/echo", ws_echo);
  tako::serve(listener, router).await;
}
```

Subprotocol negotiation, frame/message limits, allowed origins, and
ping/pong policy live on `WebSocketConfig` — pass it via
`TakoWs::with_config`. See
[`examples/websocket`](https://github.com/rust-dd/tako/tree/main/examples/websocket).

## Server-Sent Events (SSE)

`tako::sse::Sse` wraps any `Stream<Item = Into<Bytes>>` into a
`text/event-stream` body. Headers default to `no-cache` plus
`X-Accel-Buffering: no` so nginx-fronted deployments do not buffer:

```rust
use bytes::Bytes;
use futures_util::{StreamExt, stream};
use tako::Method;
use tako::responder::Responder;
use tako::router::Router;
use tako::sse::Sse;

async fn ticker() -> impl Responder {
  let s = stream::unfold(0u64, |i| async move {
    tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    Some((Bytes::from(format!("tick: {i}")), i + 1))
  });
  Sse::new(s)
}

#[tokio::main]
async fn main() {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await.unwrap();
  let mut router = Router::new();
  router.route(Method::GET, "/events", ticker);
  tako::serve(listener, router).await;
}
```

For richer events (custom `event:` names, IDs, retry hints, keep-alive
comments) use `Sse::events(stream)` with `SseEvent::data(..).event(..).id(..)`.
See [`examples/streams`](https://github.com/rust-dd/tako/tree/main/examples/streams)
and [`examples/http3-sse`](https://github.com/rust-dd/tako/tree/main/examples/http3-sse).

## PROXY protocol

When Tako sits behind an L4 load balancer (HAProxy, AWS NLB, fly.io
edges) that prepends a PROXY v1/v2 header to every TCP connection,
`spawn_proxy_protocol` parses the header and rewrites the request
extensions so handlers see the real client address:

```rust
use tako::Method;
use tako::proxy_protocol::{ProxyHeader, serve_http_with_proxy_protocol};
use tako::responder::Responder;
use tako::router::Router;
use tako::types::Request;

async fn handler(req: Request) -> impl Responder {
  let real_addr = req.extensions()
    .get::<std::net::SocketAddr>()
    .map(|a| a.to_string())
    .unwrap_or_else(|| "?".into());
  let info = req.extensions().get::<ProxyHeader>()
    .map(|h| format!("{:?}", h.transport))
    .unwrap_or_else(|| "no proxy header".into());
  format!("client={real_addr} transport={info}")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
  let listener = tokio::net::TcpListener::bind("127.0.0.1:8080").await?;
  let mut router = tako::router::Router::new();
  router.route(Method::GET, "/", handler);
  serve_http_with_proxy_protocol(listener, router).await;
  Ok(())
}
```

The parser is TLV-aware and CRC32C-verified. For high-volume L4
fronts, the standalone `tako-server-pt` crate provides an
optimised parser used by the same `spawn_proxy_protocol` entry point.
See [`examples/proxy-protocol`](https://github.com/rust-dd/tako/tree/main/examples/proxy-protocol).
