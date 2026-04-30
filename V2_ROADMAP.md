# Tako v2 Roadmap

A consolidated audit of the Tako workspace (~20k LOC, 9 crates, 38 examples, current `1.1.2`) and the concrete plan to take it to a credible 2.0.

The document is split into:

1. **Security patches** — must ship as a `1.x` release before v2 work, independent of the redesign.
2. **Core API redesign** — the breaking changes that justify a major bump.
3. **Server / transport** — production-readiness gaps and the new builder API.
4. **Plugins / middleware** — pluggable backends, missing primitives, fixes to existing plugins.
5. **Extractors / streams** — spec compliance, parity with axum, finishing half-done modules.
6. **Project hygiene** — tests, CI, docs, dependencies.
7. **Phased timeline.**

File and line references throughout point to the `feat/thread-per-code` branch as it stands today.

---

## ~~1. Security patches (ship as `1.2.0` before v2)~~ — Shipped in 1.2.0

> **Status:** All items in this section were addressed in the `1.2.0` security release. Retained here as a historical record of the audit.

### ~~1.1 Three independent insecure ID generators~~

| Location | What |
|---|---|
| ~~`tako-plugins/src/middleware/session.rs:168`~~ | ~~`generate_session_id` — deterministic LCG seeded from `SystemTime::now().nanos`. UUID-shaped, but **predictable**. Enables session-fixation.~~ |
| ~~`tako-plugins/src/middleware/csrf.rs:80`~~ | ~~`generate_csrf_token` — same LCG. Defeats the entire point of a CSRF token.~~ |
| ~~`tako-plugins/src/middleware/request_id.rs:54`~~ | ~~`generate_request_id` — same LCG. Trace IDs leak collisions.~~ |

~~**Fix:** replace all three with `uuid::Uuid::new_v4()` (already in workspace deps) or `getrandom::getrandom`.~~

### ~~1.2 Timing-oracle string compares in auth middleware~~

- ~~`api_key_auth.rs` and `bearer_auth.rs` compare credentials with `==`. Use `subtle::ConstantTimeEq`.~~

### ~~1.3 CORS credentials/wildcard footgun~~

- ~~`tako-plugins/src/plugins/cors.rs:300` reflects `*` when the configured origin set is empty, while `Access-Control-Allow-Credentials: true` is permitted alongside it. Browsers reject this combination, but the framework should refuse the configuration at build time.~~
- ~~`Access-Control-Allow-Headers: *` written literally at `cors.rs:339` regardless of `allow_credentials`.~~

### ~~1.4 HTTP/2 RST flood (CVE-2023-44487 class)~~

- ~~`tako-server/src/server_tls.rs:227` builds the H2 server with defaults: no `max_concurrent_streams`, no `max_header_list_size`, no `max_send_buf_size`. Add explicit caps and expose them in the public API.~~

### ~~1.5 HTTP/3 buffer-the-whole-body and 0-RTT replay~~

- ~~`tako-server/src/server_h3.rs:289` collects the request body into a `Vec<u8>` before dispatch. Streaming uploads over H3 are impossible.~~
- ~~`server_h3.rs:114` sets `max_early_data_size = u32::MAX` without any replay-protection wiring on the request path. Either remove this line or build the early-data extractor with explicit guidance.~~
- ~~`server_h3.rs:328` only handles `frame.data_ref()`; trailers are silently dropped.~~

### ~~1.6 PEM key formats~~

- ~~`load_key` is duplicated in `server_tls.rs:318`, `server_h3.rs:348`, `server_tls_compio.rs:296` and only accepts `pkcs8_private_keys`. RSA / SEC1 / EC keys silently fail to load. Accept all three formats and consolidate the function (see § 3).~~

### ~~1.7 Metrics cardinality~~

- ~~`tako-plugins/src/plugins/metrics.rs:230,252` use the raw URI path as a label. `/users/:id` produces a new series per ID. Switch to the matched route template (requires `MatchedPath`, see § 5).~~
- ~~`remote_addr` label on the connections counter is also unbounded.~~

### ~~1.8 Other~~

- ~~`idempotency.rs:91` defaults TTL to 30s while the docstring at `:69` advertises 24h. Pick one.~~
- ~~`idempotency.rs:380-383` ignores the configured `inflight_wait_timeout_ms` on the compio path.~~
- ~~`body_limit.rs:163-172` and `upload_progress.rs:182-229` both call `body.collect()`, defeating streaming. Replace with a `Limited<Body>` adapter.~~
- ~~`compression.rs` does not write `Vary: Accept-Encoding` and does not parse `q=0`.~~

---

## 2. Core API — Router, Handler, Macros (v2 breaking) — Done in dev branch (unreleased)

> **Status:** every subsection here is implemented on the dev branch and covered by tests; nothing is shipped to crates.io yet. The historical text is preserved with the original problem statement; each subsection is annotated with what landed.

### ~~2.1 Per-router typed state~~ — Done (runtime TypeMap variant)

~~`Router::state(value)` writes into `GLOBAL_STATE` (`tako-core/src/state.rs:44, 70`). One value per `TypeId` *per process* — two `String` configs are impossible without newtype wrappers.~~

~~**v2 design:**~~

~~```rust~~
~~let router = Router::<AppState>::new()~~
    ~~.with_state(AppState { db, cfg })~~
    ~~.get("/users/{id}", users_handler)~~
    ~~.post("/users", create_user);~~
~~```~~

~~`Router<S>` is generic over the state type; `with_state` binds it; handlers receive `State<S>` via `FromRequestParts`. Demote the global registry to an opt-in `TypeMap` helper for advanced cases.~~

**Implemented as:** runtime per-router TypeMap (`tako_core::router_state::RouterState`), not a compile-time `Router<S>` generic. `Router::with_state<T>(value)` populates the instance-local store; `State<T>` extractor reads from the request-scoped `Arc<RouterState>` first and falls back to `GLOBAL_STATE`. Two routers in the same process can hold distinct `T` values without newtype wrappers. Hot path is `AtomicBool::Acquire` fast-checked — zero overhead when `with_state` was never called. The compile-time generic was deferred (would touch every consumer crate); the API ergonomics goal is met.

### ~~2.2 `nest`, `scope`, route groups~~ — Done

~~Currently only `Router::merge` (`router.rs:871`), which mutates the shared `Arc<Route>` (`:884`). Merging the same source twice double-stacks middleware. Replace with:~~

~~```rust~~
~~let api = Router::new()~~
    ~~.nest("/v1", v1_router)~~
    ~~.nest("/v2", v2_router)~~
    ~~.scope("/admin", |r| r.layer(admin_auth).get("/", dashboard));~~
~~```~~

`Router::nest(prefix, child)` builds new `Arc<Route>` instances via `Route::cloned_with_path` so re-nesting can never double-stack the child's middleware. `Router::scope(prefix, |r| ...)` carries a `pending_prefix` field consumed by every `route() / get() / post() / …` call inside the closure. Tests cover both happy paths and the regression for the merge double-stack bug.

### ~~2.3 `Result<T, E: IntoResponse>` handler returns~~ — Done

~~Today only `Responder` is supported. Typed errors must hand-implement it. Introduce `IntoResponse` (alias of `Responder` is fine) and accept `Result<R, E: IntoResponse>` from handlers natively.~~

~~Add missing `Responder` impls: `Bytes`, `Vec<u8>`, `Cow<str>`, `serde_json::Value`, `(StatusCode, HeaderMap, Body)`, `Json<T>` shorthand.~~

`IntoResponse` is a re-export of `Responder` (no new trait subtype). The blanket `Result<T, E>` impl uses a `ResponderError` marker so it doesn't conflict with the existing `anyhow::Result<T>` impl. New `Responder` impls: `Bytes`, `Vec<u8>`, `Cow<'static, str>`, `serde_json::Value`, `(StatusCode, HeaderMap, TakoBody)`, `(StatusCode, HeaderMap)`, `HeaderMap`, `StatusCode`. `Json<T>` already implemented `Responder` previously.

### ~~2.4 405 with `Allow` header~~ — Done

~~`router.rs:489-519` returns 404 for the wrong method on a matching path. v2 should return 405 with the `Allow` header populated. Expose method introspection on the matcher.~~

`Router::collect_allowed_methods(path)` walks the `MethodMap` (cold path only; iterates the 9 standard methods). On method mismatch the response is `405 Method Not Allowed` with a comma-separated `Allow` header. Hot path unchanged. One existing test was updated from `404` → `405 + Allow`.

### ~~2.5 RFC 7807 `problem+json` default error responder~~ — Done

~~The `error_handler` hook only fires on 5xx (`router.rs:527`). Extend to 4xx and ship a default `application/problem+json` formatter.~~

New `tako::problem` module: `Problem` struct with `Responder` impl that emits `application/problem+json`, plus `default_problem_responder` helper. `Router::client_error_handler(handler)` sister to `error_handler`, fires only on 4xx. `Router::use_problem_json()` convenience installs `default_problem_responder` for both 4xx and 5xx.

### ~~2.6 Method shorthands on `Router`~~ — Done

~~Today only the macro emits typed routes. Add:~~

~~```rust~~
~~router.get(path, h);~~
~~router.post(path, h);~~
~~router.delete(path, h);~~
~~router.put(path, h);~~
~~router.patch(path, h);~~
~~```~~

`Router::get / post / put / delete / patch / head / options` shorthand methods, each `#[inline]`-forwarded to `Router::route`. Pure additive.

### ~~2.7 Drop dead code on `Route`~~ — Done

~~`Route::h09 / h10 / h11 / h2` (`route.rs:209-227`) take `&mut self`, but `Router::route` only hands back `Arc<Route>`. The methods are unreachable, and `enforce_protocol_guard` is dead. Replace with `route.version(http::Version)` using interior mutability.~~

`Route::http_protocol` is now `OnceLock<http::Version>` (mirrors the existing `timeout` / `simd_json_mode` pattern). New fluent `Route::version(http::Version) -> &Self`; `h09 / h10 / h11 / h2` are `&self` shorthands. The copy-paste `#[doc(alias = "tsr")]` on `h2` is removed.

### ~~2.8 Macro cleanup~~ — Done

~~`#[tako::route]` always emits a `*Params` struct, even for static paths (`tako-macros/src/lib.rs:209-243`). The struct name is guessed from the function name (`PascalCase + "Params"`) — rename-unsafe. Path syntax `{id: u64}` diverges from matchit/axum `{id}`.~~

~~**v2 macro:**~~
- ~~emit `*Params` only when path placeholders exist.~~
- ~~accept plain `{id}` and read the type from the handler signature.~~
- ~~align with `matchit` capture syntax.~~

`parse_path` accepts both `{id: u64}` (typed → `*Params` field) and `{id}` (untyped → matchit pass-through, no field). The `*Params` struct is only emitted when at least one typed placeholder exists. Backward-compat: `name = "..."` on a static path emits a unit-marker struct so `Name::METHOD / Name::PATH` constants stay reachable.

### 2.9 Other

- ~~`Config::from_env` (`config.rs:37`) collects `HashMap<String,String>` and serializes it through `serde_json::Value`, so non-string fields fail. Replace with `envy` or hand-rolled per-field parsing.~~ — **Done** via `envy`. Added `Config::from_env_prefixed("MYAPP_")` helper for multiple configs in one process.
- `tako-core-local` (the `!Send` router) is missing plugins, signals, OpenAPI, timeout, fallback, TSR, error_handler, and `mount_all`. Either reach parity, or document the trade-off explicitly and label it as a niche tool. — **Pending.** No work done; documenting the gap is still cheaper than parity.
- ~~`Route::h2` has `#[doc(alias = "tsr")]` (`route.rs:224`) — copy-paste bug.~~ — **Done** as part of 2.7.
- ~~`mount_all` is `linkme`-driven (`router.rs:239`) with unspecified ordering across crates and no per-prefix mount. v2: explicit `mount_all_into("/api", &mut router)`.~~ — **Done.** New `Router::mount_all_into(prefix)` consumes the same `TAKO_ROUTES` slice but prefixes every registration. Cross-crate ordering remains the linker's choice (documented).
- ~~`Router::merge` and `Route::middleware` rebuild the middleware Vec on every push (`router.rs:631-633`, `route.rs:129-131`) — racy under concurrent registration.~~ — **Done.** `Router::middleware` and `Route::middleware` switched to `ArcSwap::rcu` so concurrent pushers never lose entries (CAS retry). Hot path (`load_full` in dispatch) unchanged.

---

## 3. Server / transport (v2 breaking) — Done in dev branch (unreleased)

> **Status:** the §3 work landed on the dev branch and is covered by `cargo test` plus an end-to-end Docker bench (`Dockerfile.bench`, `Dockerfile.bench-ntex`). Subsections below are annotated with what shipped and what was deferred.

### ~~3.1 Replace the seven `serve_*` functions with a builder~~ — Done (different API shape)

~~```rust~~
~~let server = tako::Server::builder()~~
    ~~.listener(TcpListener::bind(addr).await?)~~
    ~~.http(HttpConfig::default()~~
        ~~.header_read_timeout(Duration::from_secs(30))~~
        ~~.keep_alive_timeout(Duration::from_secs(60))~~
        ~~.max_concurrent_streams(100)~~
        ~~.max_frame_size(16 * 1024)~~
        ~~.max_body_size(8 * 1024 * 1024))~~
    ~~.tls(TlsConfig::Pem { cert, key })  // or ::Resolver(Arc<dyn ResolvesServerCert>)~~
    ~~.h3(H3Config::default())~~
    ~~.limits(Limits::default()~~
        ~~.max_connections(50_000)~~
        ~~.drain_timeout(Duration::from_secs(60)))~~
    ~~.mode(Mode::PerCore { workers: num_cpus::get(), pin_cpus: true })~~
    ~~.build();~~

~~let handle = server.spawn(router);~~
~~handle.shutdown(Duration::from_secs(30)).await?;~~
~~```~~

~~This subsumes `serve`, `serve_tls`, `serve_h3`, `serve_tcp`, `serve_udp`, `serve_unix`, `serve_proxy_protocol`, all `*_with_shutdown` variants, and the separate `tako-server-pt` crate.~~

**Implemented as** (different shape, same intent):

```rust
let server = tako::Server::builder()
    .config(ServerConfig::default())          // single struct, all knobs flat
    .tls(TlsCert::pem_paths("cert.pem", "key.pem"))
    .build();

let handle = server.spawn_http(listener, router); // or spawn_tls / spawn_h2c / spawn_h3 / spawn_unix_http / spawn_proxy_protocol / spawn_tcp_raw / spawn_udp_raw
handle.shutdown(Duration::from_secs(30)).await;
```

Compio side has its own `CompioServer::builder` with `spawn_http` / `spawn_tls`.

Differences from the original design:
- listener handed to `spawn_*`, not to the builder (re-use one builder for many spawns)
- single `ServerConfig` instead of `HttpConfig` + `TlsConfig` + `H3Config` + `Limits` separation
- `TlsCert::PemPaths` only (`Der` / `Resolver` / `Acme` variants deferred)
- explicit `spawn_http` / `spawn_tls` / `spawn_h3` / etc. instead of polymorphic `spawn(router)` (compile-time clarity beats runtime switch)
- `tako-server-pt` stays a separate crate (`Mode::PerCore` integration deferred — `Box::leak` + `LocalSet` doesn't fit the tokio-runtime builder)
- runtime-agnostic `ServerHandle` (Notify-based, not `tokio::JoinHandle`) so the same type is returned by both tokio and compio paths
- `shutdown` returns `()` not `Result`

### ~~3.2 Production-readiness gaps to close~~ — Done

- ~~**`max_connections` semaphore on every transport.** `server.rs:122`, `server_tls.rs:165`, `server_unix.rs:218`, `proxy_protocol.rs:366` all unconditionally `JoinSet::spawn` per accept.~~ — **Done.** Optional `tokio::sync::Semaphore` opt-in via `ServerConfig::max_connections`. None → zero overhead.
- ~~**HTTP timeouts.** `server.rs:167-170` and `server_tls.rs:245-247` set only `keep_alive(true)` and `pipeline_flush(true)`. Wire `header_read_timeout`, `keep_alive_timeout`, H2 `keep_alive_interval`, `max_concurrent_streams`, `max_frame_size`, `initial_stream_window_size`.~~ — **Done.** `header_read_timeout`, `keep_alive`, h2 caps (`max_concurrent_streams`, `max_header_list_size`, `max_send_buf_size`, `max_pending_accept_reset_streams`, `keep_alive_interval`) all sourced from `ServerConfig`. Hyper `TokioTimer` installed on every http1 builder. `keep_alive_timeout` field reserved (hyper http1 does not yet expose the setter); `max_frame_size` / `initial_stream_window_size` similar — present in config, not yet wired.
- ~~**Tunable drain timeout.** Hardcoded 30s in seven files (`server.rs:47`, `server_tls.rs:68`, `server_h3.rs:72`, `server_unix.rs:55`, `proxy_protocol.rs:62`, `server_tcp.rs:132`, `server_compio.rs:25`).~~ — **Done.** All seven const removed; `ServerConfig::drain_timeout` is read in every transport.
- **`Box::leak(Router)`** (`server.rs:82`, `tako-server-pt/src/lib.rs:114, 318`) makes hot-reload impossible. Switch to `Arc<Router>` with RCU-style swap. — **Pending — kept on purpose.** Default path remains `&'static Router` for the per-connection / per-request hot-path performance win. Hot-reload via `arc_swap::ArcSwap<Arc<Router>>` is a separate opt-in to add later.
- ~~**Compio drain race.** `server_compio.rs:163-165` and `server_tls_compio.rs:233-235, 260-262` use `Notify::notify_one` only when `inflight == 1`. A connection finishing between the load and the await waits the full 30s. Use a `WaitGroup` or `notify_waiters` after every decrement.~~ — **Done.** `notify_waiters()` after every decrement; coordinator polls in a `while inflight > 0` loop bounded by the deadline.
- ~~**`tako-server-pt::worker_main`** is an infinite `loop { accept }` with no `select!` against shutdown (`lib.rs:132-194`); workers leak on shutdown.~~ — **Done.** New `PerThreadShutdown { Arc<Notify> }`; `spawn_per_thread` returns `(Vec<JoinHandle>, PerThreadShutdown)`; worker accept loop selects against `shutdown.notified()`.
- ~~**PROXY-protocol no read deadline** (`proxy_protocol.rs:368`). Apply `ProxyConfig::read_timeout` before parsing.~~ — **Done.** `ServerConfig::proxy_read_timeout` (default 10s); slowloris-class abuse can no longer pin a worker.
- ~~**Listener accept errors are fatal.** `server.rs:118` propagates `?`, `server_h3.rs:158` exits the listen loop on `None` from `endpoint.accept()`. Add EMFILE backoff and supervised restart.~~ — **Done.** New `tako_server::AcceptBackoff` (5 ms → 1 s exponential). Every Tokio transport (server, server_tls, server_unix, proxy_protocol) logs + backs off + retries instead of returning `?`.

### 3.3 Extract a `tako-tls` crate — Done (in-tree module, no separate crate)

`load_certs` and `load_key` were duplicated in `server_tls.rs:318-362`, `server_h3.rs:348-367`, `server_tls_compio.rs:296-315`, and `tako-streams/src/webtransport.rs:170-194` reaches across crates into `tako_server::server_h3::load_certs`. ~~Move to a shared `tako-tls` crate exposing:~~

```rust
pub enum TlsConfig {
    Pem { cert: Vec<u8>, key: Vec<u8> },
    Der { cert: Vec<CertificateDer<'static>>, key: PrivateKeyDer<'static> },
    Resolver(Arc<dyn ResolvesServerCert>),
    Acme { directory_url: String, contact: Vec<String>, cache_dir: PathBuf },
}
```

~~Support PKCS#8, RSA, SEC1, EC.~~ Add SNI multi-cert resolver. Wire mTLS via `WebPkiClientVerifier`. Add hot reload (file-watcher or signal-driven).

**Done partially:** `load_certs` / `load_key` consolidated into `tako_core::tls` (PKCS#8 + RSA + SEC1 + EC accepted). `server_tls`, `server_h3`, `server_tls_compio` re-export from there; `webtransport` calls it directly so it no longer reaches across crates. New workspace member was avoided (kept the crate count down). The `TlsConfig` enum + SNI multi-cert + mTLS + hot-reload pieces are deferred to the `Server::builder` `TlsCert` evolution.

### 3.4 Protocol-completeness items

- **HTTP/3:** ~~stream the request body, support trailers~~, support graceful GOAWAY (currently `endpoint.close(0u32.into(), ...)` is hard-close at `server_h3.rs:204`), expose qlog, retry-token, datagrams, congestion-control selection, max bidi/uni streams. — **Streaming body + trailers Done** (per `build_h3_body`). GOAWAY / qlog / retry-token / datagrams / congestion-control selection / max bidi/uni streams **Pending.**
- ~~**h2c (cleartext H2)** for L7-proxy deployments.~~ — **Done.** New `tako_server::server_h2c` module; `Server::spawn_h2c(listener, router)` in the builder.
- ~~**80→443 auto-redirect helper.**~~ — **Done.** `tako::redirect::http_to_https_router(https_port)` builds a router whose fallback 308-redirects every request.
- **socket activation** (`LISTEN_FDS`). — **Pending.**
- **abstract Unix sockets** (`@`-prefixed). — **Pending.**
- **vsock** for VM-host bridges. — **Pending.**
- ~~**PROXY v2 TLV parsing** (`proxy_protocol.rs:225-309`): AWS VPC endpoint ID (0xEA), TLS info (0x20), authority (0x02), CRC32C. Strip inbound `X-Forwarded-For` before injecting source. Handle `AF_UNIX` family (0x3) — currently silently lands in `_ => UNSPEC` at `:301-308`.~~ — **Done** (CRC32C parsed but not yet verified). New `ProxyHeader` fields: `authority`, `alpn`, `aws_vpc_endpoint_id`, `tls: Option<ProxyTlsInfo>`, `unique_id`, `tlvs: Vec<ProxyTlv>`, `source_unix`, `destination_unix`. Inbound `X-Forwarded-*` and `Forwarded` headers stripped in the service_fn before dispatch.
- ~~**Unify `ConnInfo` extension.** `server.rs:139` and `server_tls.rs:196` insert `SocketAddr`; `server_unix.rs:222` inserts `UnixPeerAddr`; H3 inserts something else again. Define one struct:~~

```rust
pub struct ConnInfo {
    pub peer: PeerAddr,             // IP, Unix, vsock, ...
    pub local: PeerAddr,
    pub transport: Transport,        // Tcp, Tls, H3, Unix, ...
    pub alpn: Option<Vec<u8>>,
    pub sni: Option<String>,
    pub tls_version: Option<TlsVersion>,
    pub proxy_header: Option<ProxyHeader>,
}
```

— **Done.** New `tako_core::conn_info` module: `ConnInfo { peer, local, transport, tls }`, `PeerAddr { Ip, Unix, Other }`, `Transport { Http1, Http2, Http3, Unix, Tcp }`, `TlsInfo { alpn, sni, version }`. Every transport (TCP/TLS/H3/Unix/PROXY/h2c/per-thread, both tokio and compio) inserts a `ConnInfo` into request extensions alongside the legacy `SocketAddr` / `UnixPeerAddr` / `ProxyHeader` for backward compat.

### ~~3.5 Observability inside the server crate~~ — Done (per-request part); traceparent pending

~~The four `signals` emissions (`server.rs:122-191`, `server_tls.rs:165-265`, `server_h3.rs:161-194, 273-318`) are copy-pasted. Move emission into a single per-request middleware so transport files don't duplicate the boilerplate.~~ Wire W3C `traceparent` propagation.

`REQUEST_STARTED` / `REQUEST_COMPLETED` emit moved into `Router::dispatch` (one site, all transports get it for free; gated on the `signals` feature). Connection-level emit factored into `tako_core::signals::transport::{emit_server_started, emit_connection_opened, emit_connection_closed}`. Five transport files lost ~5–10 lines each of boilerplate. **W3C `traceparent` propagation is pending** — see §4.2 `request_id` and the `traceparent` middleware in §4.3.

---

## 4. Plugins / middleware (v2)

### 4.1 Pluggable backends

Today every store is `scc::HashMap`. Define traits and ship `Memory*` + feature-gated `Redis*` (and optionally `Postgres*`) implementations:

```rust
trait SessionStore: Send + Sync + 'static { ... }
trait RateLimitStore: Send + Sync + 'static { ... }
trait IdempotencyStore: Send + Sync + 'static { ... }
trait JwksProvider: Send + Sync + 'static { ... }
trait CsrfTokenStore: Send + Sync + 'static { ... }
```

### 4.2 Existing plugin fixes

| Plugin | Fix |
|---|---|
| `session` | Rotate on privilege change; split idle vs absolute timeout; rolling cookie refresh on every request (currently set only when `is_new`, `session.rs:267-292`); revoke-all helper. |
| `rate_limiter` | Per-route / per-user / per-IP composite key; emit `RateLimit-Limit / RateLimit-Remaining / RateLimit-Reset` and `Retry-After` (draft-ietf-httpapi-ratelimit-headers); GCRA option. Currently per-IP only with fallback `0.0.0.0` collapsing all unknown clients into one bucket (`rate_limiter.rs:406-410`). |
| `idempotency` | Reconcile docstring vs default TTL; respect `inflight_wait_timeout_ms` on compio (`idempotency.rs:380-383`); cap stored response size. |
| `jwt_auth` | JWKS rotation, asymmetric keys, configurable `iss` / `aud` / `kid` / leeway, revocation list, optional remote introspection. |
| `csrf` | Bind token to session; origin/referer fallback; relax `SameSite` to a configurable choice. |
| `compression` | Write `Vary: Accept-Encoding`; parse `q=0`; cap inbound decompression (compression-bomb defense); content-type allow-list as configurable enum, not substring match. |
| `cors` | Refuse `Allow-Credentials: true` with reflective wildcard at config build time; regex/suffix origin matching; PNA support. |
| `metrics` | Use `MatchedPath` for the route label; switch to histograms; configurable bucket schedule; drop `remote_addr` label. |
| `body_limit` | Stream-aware limit, no full `body.collect()`. |
| `upload_progress` | Stream-aware, no full buffering; abandonment cleanup on disconnect. |
| `security_headers` | CSP nonce/hash support; COOP / COEP / CORP; Permissions-Policy; remove `X-XSS-Protection: 0`. |
| `request_id` | W3C `traceparent` parsing and emission. |

### 4.3 Missing middleware to add for v2

- `timeout` — per-request deadline.
- `traceparent` propagation (W3C trace context).
- `access_log` — structured access log separate from metrics.
- `problem+json` error responder.
- `circuit_breaker` and outbound `retry` for the client.
- `ip_filter` — allow/deny + CIDR.
- `healthcheck` — readiness/liveness + drain semantics.
- `etag` / conditional GET helper.
- `tenant` — `X-Tenant-ID` extraction with scoped state.
- `hmac_signature` — Stripe/AWS-style request signing.
- `json_schema` — request/response validator.

---

## 5. Extractors / streams (v2)

### 5.1 Extractors

- **Finish or remove `zero_copy_extractors`.** `tako-extractors/src/zero_copy_extractors.rs` is three lines (`pub mod` declarations) and the README advertises a `zero-copy-extractors` feature flag. Either build it out or delete the feature.
- **axum parity:** `TypedHeader<H>`, `Extension<T>`, `MatchedPath`, `OriginalUri`, `Host`, `Scheme`, `ConnectInfo<T>`, `ContentLengthLimit<T, N>`.
- `Path<T>`: support nested types, tuples, `Vec`, `Option`.
- `Query<T>`: repeated keys / arrays / CSV.
- `Multipart`: per-part max size, content-type allow-list, disk-spill threshold, max parts.
- `JwtClaims<T>`: today only base64-decodes (`jwt.rs`). Either rename to `JwtClaimsUnverified` to make the trust model explicit, or perform verification in the extractor with a `JwksProvider` from state.
- Cookies: key-id metadata for rotation; encryption rotation across `cookie_private` / `cookie_signed`.
- Validation integration with `validator` or `garde` as an opt-in feature.

### 5.2 Streams

**SSE (`tako-streams/src/sse.rs`)** is currently spec-partial:
- supports only the `data:` field. Add `event:`, `id:`, `retry:` fields and a builder API.
- support `Last-Event-ID` replay (caller-provided closure).
- emit periodic comment frames (`:keepalive\n\n`) for proxy keep-alive.
- send `X-Accel-Buffering: no` to defeat nginx buffering by default.

**WebSocket (`tako-streams/src/ws.rs`)** is currently a thin upgrade helper:
- echo `Sec-WebSocket-Protocol` (subprotocol negotiation).
- ping/pong with configurable interval and timeout.
- `permessage-deflate` extension.
- `max_frame_size` and `max_message_size` config.
- Origin allowlist.
- upgrade timeout — `ws.rs:164` spawns `tokio::spawn` waiting on `on_upgrade.await` with no deadline; if the client never upgrades the task leaks.
- target Autobahn green.

**File stream (`file_stream.rs`)** has range support but lacks:
- `multipart/byteranges`.
- ETag, `If-Modified-Since`, `If-None-Match`.
- zero-copy `sendfile` path on Linux.

**Static (`tako-streams/src/static.rs`)** has a single fallback file but lacks:
- precompressed file preference (`*.br`, `*.gz` next to the original).
- SPA fallback as a rewrite (current single fallback is not the same).
- explicit canonicalize + prefix check for path traversal.
- index resolution priority list.

**WebTransport (`webtransport.rs:170`)** reaches across crates and **does not perform the CONNECT handshake** — what is exposed today is raw QUIC, which is not WebTransport per the W3C draft. Implement the CONNECT extended handshake or downgrade the docs.

### 5.3 gRPC

The current implementation (`tako-core/src/grpc.rs`) is unary only. Add:
- client streaming, server streaming, bidirectional streaming.
- `grpc.reflection.v1` server reflection.
- `grpc.health.v1` health service.
- gRPC-Web bridge.
- `grpc-timeout` deadline propagation into request extensions.
- gRPC-specific interceptor / middleware story (current HTTP middleware semantics don't fit cleanly).

### 5.4 GraphQL

- persisted queries.
- complexity / depth / cost limits.
- dataloader integration documented.

### 5.5 OpenAPI

`utoipa` and `vespera` coexist (`tako-core/src/openapi/{utoipa,vespera}.rs`). Pick one as primary and demote the other to opt-in, or build a thin discovery layer over both. Today both are exposed through feature flags with overlapping responsibilities.

### 5.6 Core platform — queue, signals, client

**Queue (`tako-core/src/queue.rs`)** is in-memory only. DLQ is in-memory too; restart loses jobs. v2 minimum:

```rust
trait QueueBackend: Send + Sync + 'static {
    async fn push(&self, queue: &str, payload: &[u8], opts: PushOptions) -> Result<JobId>;
    async fn reserve(&self, queue: &str) -> Result<Option<ReservedJob>>;
    async fn complete(&self, id: JobId) -> Result<()>;
    async fn fail(&self, id: JobId, retry_at: Option<Instant>) -> Result<()>;
    async fn dead_letter(&self, id: JobId) -> Result<()>;
}
```

with `MemoryBackend`, `RedisBackend`, optionally `PostgresBackend` (LISTEN/NOTIFY). Add idempotent dedup keys, cron scheduling, observability hooks.

**Signals (`tako-core/src/signals.rs`)** are process-local and lossy (broadcast drop-on-slow-consumer). v2: filtered subscriptions, optional cluster-scope (Redis pub/sub), and a consistent naming scheme — current ids mix `request.started`, `request.completed`, `route.request.started`.

**Client (`tako-core/src/client.rs`)** is HTTP/1.1 only with one TCP connection per `TakoClient`. No pool, no retry, no timeout, no cancellation, no tracing propagation. For v2 either rebuild on `hyper-util` legacy client with full pool/H2/H3/timeout/retry semantics, or re-export `reqwest` behind the `client` feature and keep the trivial helper as a learning example.

---

## 6. Project hygiene

### 6.1 Tests

- **0 unit tests inside any `src/` file** across all crates.
- All tests live in `tako-rs/tests/` — 9 integration files, ~125 tests:
  - `middleware.rs` (31), `router.rs` (19), `extractors.rs` (17), `queue.rs` (13), `udp_tcp_progress.rs` (12), `typed_routes.rs` (11), `responder.rs` (10), `sse_redirect_config.rs` (10), `mount_all.rs` (2).
- No property tests, no fuzz, no Miri runs.
- No criterion benches (only the wrk-driven `examples/bench-*`).

**v2 target:**
- 70% line coverage on `tako-core`, `tako-extractors`, `tako-plugins`.
- Fuzz harnesses on every parser: PROXY v1 and v2, multipart, JSON, URL-encoded form, JWT, cookies.
- Miri pass on `tako-core` and `tako-extractors`.
- Autobahn WebSocket suite green.
- Criterion benches for the hot path with regression gating.

### 6.2 CI

The current `.github/workflows/ci.yml` runs only:

```yaml
- cargo build --release
- cargo build --release --all-features
- cargo build --release --examples
```

There is **no `cargo test`, no clippy, no fmt-check, no doctest, no MSRV, no Miri, no sanitizer, no coverage, no platform matrix**. Additionally, `cargo build --examples` from the workspace root effectively builds nothing because all 38 example crates are in the workspace `exclude:` list — example breakage is not detected.

**v2 minimum CI:**

```yaml
matrix:
  os: [ubuntu-latest, macos-latest, windows-latest]
  toolchain: [stable, "1.87.0", beta]
steps:
  - cargo fmt --all -- --check
  - cargo clippy --all-features --workspace -- -D warnings
  - cargo test --all-features --workspace
  - cargo doc --no-deps --all-features  # with -D rustdoc::broken_intra_doc_links
  - cargo +nightly miri test -p tako-core -p tako-extractors
  - cargo deny check
  - cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
  - examples build job that iterates over examples/*/Cargo.toml and builds each
  - criterion benchmark gate (on PRs touching tako-core / tako-server)
```

### 6.3 Documentation

- No mdbook, no migration guide, no API stability statement.
- Rustdoc is uneven across crates; many extractor and middleware modules have generous docstrings while server transport files are sparse.
- `lib.rs` of `tako-rs` is the public re-export crate but the navigation through feature flags is hard.

**v2 docs deliverables:**
- `MIGRATION_1_TO_2.md` covering every breaking change in §§ 2-5.
- mdbook with: getting-started, transports overview, routing, state, middleware, extractors, streams, queue, signals, observability, deployment.
- API stability policy (which re-exports are stable, what semver guarantees we make on the global `signals` ids, etc.).

### 6.4 Dependencies

- `sonic-rs` **and** `simd-json` are both pulled in (`Cargo.toml:126, 127`). Pick one. `simd-json` is more portable; `sonic-rs` is faster on x86_64 with AVX2.
- `webpki-roots` (frozen snapshot) is fine for hermetic builds; consider `rustls-native-certs` as a feature for users who want the system trust store.
- `send_wrapper` is used to satisfy hyper's `Send` bound on compio H2 timers (`server_tls_compio.rs:380-405`). Document this as a hard invariant: the `Send` claim is per-runtime, not global.
- `linkme` powers `mount_all` with unspecified ordering; either accept it and document, or replace with explicit registration.

---

## 7. Phased roadmap

| Phase | Scope | Estimated effort (one engineer) |
|---|---|---|
| ~~**`1.2.0` security release**~~ | ~~§ 1 in full. Blog post documenting the audit.~~ — **Shipped.** | ~~1 week~~ |
| **v2 alpha — core** | § 2: `Router<S>`, `IntoResponse`, `Result<_, E>`, `nest`/`scope`, 405+`Allow`, RFC 7807, macro cleanup, `mount_all` redesign, `tako-core-local` parity decision. | 3-4 weeks |
| **v2 alpha — server** | § 3: `Server::builder`, `tako-tls` crate, `Arc<Router>` (drop `Box::leak`), `Limits` + `HttpConfig`, unified `ConnInfo`, `tako-server-pt` merge, h2c, H3 streaming body, PROXY v2 TLV, mTLS hooks. | 3-4 weeks |
| **v2 alpha — plugins** | § 4: backend traits, `RedisStore`, `timeout`, `traceparent`, `problem+json`, `healthcheck`, `ip_filter`, `etag`, fixes to existing plugins. | 2-3 weeks |
| **v2 alpha — streams + extractors** | § 5: SSE spec compliance, WS subprotocol/ping/permessage-deflate + Autobahn, `TypedHeader`/`Extension`/`MatchedPath`, validator integration, finish or delete `zero_copy_extractors`. | 2-3 weeks |
| **v2 beta — hygiene** | § 6: tests to 70%, fuzz harnesses, Miri, full CI matrix, mdbook, migration guide, example fleet rebuild. | 2 weeks |
| **v2.0 release** | Ship. | — |

Total: **~12-16 weeks for one engineer**, **~6-8 weeks for two**.

~~The `1.2.0` security release should ship **before** any v2 work begins, both because the bugs are real and because a public audit blog post is an effective lead-in to a v2 announcement.~~ — **Done; v2 work can begin.**
