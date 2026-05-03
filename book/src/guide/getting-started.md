# Getting started

> **Status:** scaffold — full content lands as part of the 2.0 docs
> sweep.

## Install

```toml
[dependencies]
tako-rs = { version = "2", features = ["tls", "http2"] }
tokio = { version = "1", features = ["full"] }
```

## Hello world

```rust,ignore
use tako::router::Router;
use tako::Server;
use tokio::net::TcpListener;

#[tokio::main]
async fn main() {
    let mut router = Router::new();
    router.get("/", |_req: tako::types::Request| async { "hello" });

    let listener = TcpListener::bind("0.0.0.0:8080").await.unwrap();
    let server = Server::builder().build();
    let handle = server.spawn_http(listener, router);

    tokio::signal::ctrl_c().await.unwrap();
    handle.shutdown(std::time::Duration::from_secs(30)).await;
}
```

For more involved examples (TLS, HTTP/3, gRPC, WebTransport, …) see
the [transports overview](./transports.md) and the `examples/`
directory in the repository.
