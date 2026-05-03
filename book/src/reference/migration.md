# Migrating from Tako 1.x to 2.0

> **Status:** living document. Tracks every breaking change between the
> released `1.x` line on crates.io and the unreleased 2.0 work on `main`.
> Sections marked **(deferred)** are not yet on `main` and will land before
> the 2.0 release tag.

This guide is paired with [`V2_ROADMAP.md`](./V2_ROADMAP.md). The roadmap
explains *why* a change happened; this guide explains *what to change in
your code*.

## At a glance

```text
┌────────────────────────────────────────────────────────────────────────────┐
│ Area              │ 1.x                          │ 2.0                     │
├────────────────────────────────────────────────────────────────────────────┤
│ Per-router state  │ GLOBAL_STATE, one per type   │ Router::with_state      │
│ Handler returns   │ Responder only               │ Result<R, E: Responder> │
│ Sub-routing       │ Router::merge                │ nest(), scope()         │
│ Wrong-method      │ 404 Not Found                │ 405 + Allow header      │
│ Errors            │ error_handler (5xx only)     │ + client_error_handler  │
│                   │                              │ + use_problem_json()    │
│ Macros            │ {id: u64} only, always Param │ {id} + {id: u64}, no    │
│                   │                              │ Params struct unless    │
│                   │                              │ typed slot exists       │
│ Server bootstrap  │ serve_*, serve_tls, …        │ Server::builder()       │
│ TLS knobs         │ Pem only                     │ Pem + Der + Resolver +  │
│                   │                              │ ReloadableResolver +    │
│                   │                              │ ClientAuth (mTLS)       │
│ Connection info   │ SocketAddr / UnixPeerAddr    │ ConnInfo (unified)      │
│ tako-core-local   │ separate !Send router        │ removed                 │
└────────────────────────────────────────────────────────────────────────────┘
```

## 1. Per-router typed state

**1.x**

```rust
let cfg = Config { db, secret };
Router::state(cfg);                    // one slot per TypeId, global
let router = Router::new();
```

**2.0**

```rust
let router = Router::new()
    .with_state(Config { db, secret });  // instance-local
// `State<T>` extractor reads from the per-router store first, then falls
// back to GLOBAL_STATE for backward compat.
```

Two routers in the same process can hold distinct `T` values without
newtype wrappers. Hot-path overhead is one `AtomicBool::Acquire` when the
feature is unused.

## 2. Handler return types

**1.x**

```rust
async fn handler() -> impl Responder { ... }
```

**2.0** — `Result<T, E: IntoResponse>` is supported natively:

```rust
async fn handler() -> Result<Json<User>, ApiError> { ... }
```

`IntoResponse` is a re-export of `Responder`. New blanket impls were added
for `Bytes`, `Vec<u8>`, `Cow<'static, str>`, `serde_json::Value`,
`(StatusCode, HeaderMap, TakoBody)`, `(StatusCode, HeaderMap)`,
`HeaderMap`, and `StatusCode`.

## 3. Sub-routing — `nest` and `scope`

**1.x**

```rust
let api = Router::new();
let main = Router::new();
main.merge(api);   // mutates shared Arc<Route>; double-merging stacks
                   // middleware twice
```

**2.0**

```rust
let main = Router::new()
    .nest("/v1", v1_router)
    .nest("/v2", v2_router)
    .scope("/admin", |r| {
        r.layer(admin_auth).get("/", dashboard);
    });
```

`nest` clones routes via `Route::cloned_with_path` so re-nesting can
never double-stack middleware. `scope` carries a pending prefix consumed
by every method shorthand inside the closure.

## 4. 405 with `Allow` instead of 404

**1.x** returned `404 Not Found` when the path matched but the method
did not. **2.0** returns `405 Method Not Allowed` with a comma-separated
`Allow` header.

If you have tests asserting `404` on a path-match-method-miss, update
them to expect `405` plus the appropriate `Allow` value.

## 5. RFC 7807 `application/problem+json`

**1.x** had `Router::error_handler(...)` that fired only on 5xx.

**2.0** adds:

- `Router::client_error_handler(...)` — fires on 4xx.
- `Router::use_problem_json()` — convenience that installs
  `default_problem_responder` for both 4xx and 5xx.
- `tako::problem::Problem` struct with a `Responder` impl that emits
  `application/problem+json`.

## 6. Macro syntax

**1.x**

```rust
#[tako::route("GET", "/users/{id: u64}")]
async fn get_user(id: u64) -> ... { ... }
// Always emits `GetUserParams` struct, even on static paths.
```

**2.0**

- `{id: u64}` and `{id}` are both accepted. The first is a typed slot, the
  second is `matchit` pass-through.
- The `*Params` struct is **only** emitted when at least one typed slot
  exists.
- For static paths with `name = "..."`, a unit-marker struct is still
  emitted so `Name::METHOD` / `Name::PATH` constants stay reachable.

## 7. Server bootstrap

**1.x**

```rust
serve(router, addr).await?;
serve_tls(router, addr, cert, key).await?;
serve_h3(router, addr, cert, key).await?;
serve_unix(router, path).await?;
serve_proxy_protocol(...).await?;
```

**2.0**

```rust
let server = tako::Server::builder()
    .config(ServerConfig::default()
        .header_read_timeout(Duration::from_secs(30))
        .keep_alive(true)
        .max_concurrent_streams(100)
        .max_connections(50_000)
        .drain_timeout(Duration::from_secs(60)))
    .tls(TlsCert::pem_paths("cert.pem", "key.pem"))
    .build();

let handle = server.spawn_http(listener, router);
// .spawn_tls / .spawn_h2c / .spawn_h3 / .spawn_unix_http /
// .spawn_proxy_protocol / .spawn_tcp_raw / .spawn_udp_raw

handle.shutdown(Duration::from_secs(30)).await;
```

Changes from the original v2 roadmap shape:

- The listener is handed to `spawn_*`, not the builder, so a single
  `Server` instance can fan out to multiple listeners.
- `ServerConfig` is one flat struct instead of `HttpConfig` +
  `TlsConfig` + `H3Config` + `Limits`.
- `ServerHandle::shutdown` returns `()` and is runtime-agnostic
  (Notify-based) so the same type comes back from both tokio and
  compio paths.

## 8. TLS

**1.x** supported `TlsCert::PemPaths`. **2.0** adds:

- `TlsCert::Der { certs, key, client_auth }`
- `TlsCert::Resolver { resolver, client_auth }`
- `ClientAuth::{Optional(roots), Required(roots)}` for mTLS, threaded
  through every TCP/TLS, compio-TLS, and HTTP/3 spawn path.
- `ReloadableResolver` for hot-reload without a listener restart
  (callers wire their own file-watcher / signal trigger).
- New entry points `serve_tls_with_rustls_config_and_shutdown` and
  `serve_h3_with_rustls_config_and_shutdown` that take a fully-built
  `Arc<rustls::ServerConfig>` for advanced cases.

ACME (`TlsCert::Acme { ... }`) is **deferred**.

## 9. Trust store: `webpki-roots` vs OS

**2.0** adds an opt-in `native-certs` feature that swaps the bundled
`webpki-roots` snapshot for `rustls-native-certs` (operating-system
trust store). Default behavior is unchanged — `webpki-roots` is still
used unless `native-certs` is enabled.

```toml
tako-rs = { version = "2", features = ["client", "native-certs"] }
```

## 10. Unified `ConnInfo`

**1.x** inserted a different connection-info type per transport:
`SocketAddr` (TCP/TLS), `UnixPeerAddr` (Unix), something else for H3.

**2.0** unifies on:

```rust
struct ConnInfo {
    peer: PeerAddr,        // Ip / Unix / Other
    local: PeerAddr,
    transport: Transport,  // Http1 / Http2 / Http3 / Unix / Tcp
    tls: Option<TlsInfo>,  // alpn, sni, version
}
```

Legacy types (`SocketAddr`, `UnixPeerAddr`, `ProxyHeader`) remain in
extensions for backward compatibility, alongside `ConnInfo`.

## 11. Plugin / middleware updates

| Plugin / middleware | 2.0 change |
|---|---|
| `session` | Idle vs absolute TTL, rolling cookie refresh, `Session::rotate()`, configurable `SameSite`/`Domain`, bulk revocation |
| `rate_limiter` | Composite-key support, IETF `RateLimit-*` headers, `Algorithm::Gcra`, `UnkeyedBehavior` choice |
| `idempotency` | Verified TTL = 86_400 s, compio `inflight_wait_timeout_ms` honored |
| `jwt_auth` | Iss/aud/leeway constraints, `MultiKeyVerifier`, runtime rotation/revocation, optional remote introspection |
| `csrf` | Token bound to `Session`, Origin/Referer allow-list, configurable `SameSite` |
| `compression` | `ContentTypePolicy` enum replaces substring filter |
| `cors` | `OriginMatcher::{Exact, Suffix, Custom}`, `allow_private_network` |
| `metrics` | Latency histogram with `with_buckets(..)` override |
| `security_headers` | CSP + nonce, COOP/COEP/CORP, Permissions-Policy, HSTS preload toggle, `X-XSS-Protection` removed |
| `request_id` | Now focused on `X-Request-ID`; `traceparent` parsing moved to a new middleware |
| **NEW**: `timeout` | Per-request deadline, dynamic-per-request override |
| **NEW**: `traceparent` | W3C Trace Context parser/emitter, `TraceContext` extension |
| **NEW**: `access_log` | Structured one-line access log; default sink `tracing` INFO |
| **NEW**: `problem+json` | Rewrites non-JSON 4xx/5xx into `application/problem+json` |
| **NEW**: `circuit_breaker` | Closed/open/half-open with rolling counter |
| **NEW**: `ip_filter` | CIDR allow/deny lists |
| **NEW**: `healthcheck` | `/live`, `/ready`, `/__drain` with async readiness probes |
| **NEW**: `etag` | SHA-1 strong validator, conditional GET |
| **NEW**: `tenant` | `X-Tenant-ID` / subdomain / path-segment / custom strategies |
| **NEW**: `hmac_signature` | HMAC-SHA256 signature verification |
| **NEW**: `json_schema` | Request/response validator |

## 12. Backend traits

`tako_plugins::stores` adds five traits:

- `SessionStore`
- `RateLimitStore`
- `IdempotencyStore`
- `JwksProvider`
- `CsrfTokenStore`

Built-in middleware still defaults to in-memory stores. Implement these
traits to back middleware with Redis / Postgres / external services.

> Companion crates `tako-stores-redis` and `tako-stores-postgres` are on
> the §4.1 follow-up list and intentionally not part of the framework
> dependency surface.

## 13. Extractors

- `tako_extractors::Path<T>` is the new axum-style wrapper. The old
  zero-arg extractor was renamed `RawPath` (**breaking**). Migrate any
  call site that used `Path` as a no-argument extractor.
- `JwtClaims<T>` is renamed `JwtClaimsUnverified<T>`. The old name
  remains as a `#[deprecated]` alias. The verifying counterpart is
  `tako_plugins::extractors::jwt::JwtClaimsVerified<C>`, fed by
  `JwtAuth<V>` middleware.
- New: `TypedHeader<H>` (feature `typed-header`), `Extension<T>`,
  `MatchedPath`, `OriginalUri`, `Host`, `Scheme`, `ConnectInfo<T>`,
  `ContentLengthLimit<T, N>`, `QueryMulti<T>`, `MultipartConfig`-driven
  `BufferedUploadedFile`, `KeyRing`-rotated cookie extractors,
  `Validated<T>` (features `validator` / `garde`).

## 14. `tako-core-local` removed

The `!Send` `LocalRouter` was removed. Per-worker isolation is fully
covered by `serve_per_thread` / `serve_per_thread_compio` with the
thread-safe `Router`. Replace any `tako::local::*` import with the
matching thread-safe equivalent. The `per-thread-local` /
`per-thread-compio-local` cargo features are gone.

## 15. Streams

- `Sse` gained `SseEvent` builder, `Sse::events(...)`,
  `Sse::keep_alive(...)`, `last_event_id(headers)` helper.
- `TakoWs<H>` builder gained `protocols`, `max_frame_size`,
  `max_message_size`, `allowed_origins`, `upgrade_timeout`,
  `keep_alive(WsKeepAlive)`. `permessage-deflate` is exposed via
  `WebSocketConfig`. Autobahn green and built-in deflate are deferred.
- `FileStream::with_etag(..)`, `with_last_modified(..)`,
  `with_content_type(..)`, `evaluate_conditional(...)`,
  `weak_etag_from_metadata(...)`. Multipart/byteranges and `sendfile(2)`
  are deferred.
- `ServeDirBuilder::precompressed(...)`, `index_files([...])`, traversal
  rejection at parse time. SPA fallback uses the same resolver.
- WebTransport is currently raw QUIC and is also exported as
  `RawQuicSession` so call sites can pick the honest name. The W3C
  WebTransport CONNECT handshake is deferred.

## 16. gRPC

- New: `GrpcServerStream<S, T>`, `GrpcClientStream<T>`,
  `GrpcBidi<Req, Resp>`.
- `parse_grpc_timeout(...)` and `read_grpc_deadline(req)` insert a
  `GrpcDeadline(Instant)` extension.
- `GrpcInterceptor` async trait + `InterceptorChain` short-circuiting on
  the first `Err(GrpcStatus)`.
- Reflection / health scaffolding (storage layer ships now; protobuf
  encoders deferred until consumers don't have to ship `protoc`).
- gRPC-Web byte-level decoder/encoder helpers.

## 17. GraphQL & OpenAPI

- APQ via `PersistedQueryStore` trait + `MemoryPersistedQueryStore`.
- Complexity / depth / cost limits builder on `async_graphql::SchemaBuilder`.
- `utoipa` is now the documented primary OpenAPI integration; `vespera`
  remains available behind its existing cargo feature.

## 18. Queue & signals

- `QueueBackend` async trait. `MemoryBackend` ships in-tree; remote
  brokers go in companion crates.
- `Queue::push_dedup(name, payload, key)` collapses duplicate pending
  jobs.
- Cron scheduling behind the `queue-cron` cargo feature.
- New signals: `queue.job.queued / started / completed / failed /
  retrying / dead_letter`. Canonical strings under
  `tako_core::queue::signal_ids`.
- `signals::bus::SignalBus` async trait + `LocalBus` no-op default for
  cluster-wide signal forwarding.

## 19. v2 client

`V2Client` + `V2ClientBuilder` ride on `hyper_util::client::legacy::Client`:

- connection pool with idle timeout / per-host caps
- per-request timeout
- retry policy with exponential backoff
- default `User-Agent`, `traceparent`-friendly request handling

The legacy `TakoClient` / `TakoTlsClient` keep working for backward
compatibility but new code should use `V2Client`.

## Known incompatibility: `--all-features` does not compile

Enabling **both** the tokio and compio runtimes simultaneously
(`cargo build --all-features`) is not supported. The `compio::time::sleep`
future is `!Send`, while hyper's service bound (used for tokio
transports) is `Send`. The middleware that bridges them — currently
`tako_plugins::middleware::timeout::Timeout` — picks one runtime per
build via `#[cfg(...)]`.

If you need both runtimes in the same process, build separate binaries
or hand-pick a feature subset that activates only one runtime side.

## Roadmap items intentionally not in this guide

The following are tracked in [`V2_ROADMAP.md`](./V2_ROADMAP.md) and have
**not yet** landed on `main`:

- `tako-stores-redis`, `tako-stores-postgres`
- `TlsCert::Acme { ... }`
- HTTP/3 qlog
- Multipart/byteranges responder, Linux `sendfile(2)` path
- Real WebTransport CONNECT handshake
- Generated gRPC stubs (reflection / health protobuf)
- Cluster `SignalBus` impls (Redis, NATS, Kafka)
- HTTP/2 + HTTP/3 + reqwest-style middleware on the v2 client
- Hot-reload `Arc<Router>` swap (current path keeps `Box::leak` for
  per-connection performance)

When these land, this guide is updated alongside.
