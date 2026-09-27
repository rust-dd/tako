# Changelog

All notable changes to **tako-rs** are documented here. Format inspired by
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

The planned 2.1 release includes intentional breaking API and default changes.

### Security

- Updated the dependency graph to patched h2 and rustls releases. Dependency
  advisories now cover every feature; obsolete exceptions were removed.
- Trusted-proxy IP extraction supports router-local CIDR policies, repeated
  forwarding headers, and rejects malformed or opaque hops without trusting
  client-supplied fallback headers.

- Buffered JSON, form, protobuf, SIMD, borrowed, bytes and text extractors default
  to a 2 MiB limit. Configure `Router::body_limit` or explicitly opt out with
  `disable_body_limit`. Exceeding either this limit or a `BodyLimit` middleware
  wrapper returns 413, including for chunked bodies.
- `anyhow::Error` responses return a generic 500 and log diagnostic details.
  Text and binary responders set explicit content types.
- Plugin setup errors are retained and reject requests; server initialization
  propagates router plugin failures.

### Changed

- `TlsInfo` uses shared `Bytes` for ALPN and `Arc<str>` for SNI. The ineffective
  `ServerConfig::keep_alive_timeout` field was removed; use the implemented
  keep-alive toggle and request-header deadline. `PerThreadConfig` adds
  `header_read_timeout` and a per-worker `max_connections` limit.

- Updated public integrations to tungstenite 0.30, Prometheus 0.14, OpenTelemetry
  0.33, validator 0.21, garde 0.23 and utoipa 6. Applications sharing these
  dependency types must update their matching dependencies. Async-graphql uses
  stable 7.2.1. Compio/cyper, SIMD JSON, JSON Schema and compression libraries
  were also updated; PEM loading uses Rustls `PemObject` directly.

- Only the last handler argument may consume the body. Earlier arguments must
  implement `FromRequestParts`. Extractor futures no longer allocate a box.
- `Route::path`, `MatchedPath` and captured parameter keys use `Arc<str>`.
  Use `.as_ref()` to borrow strings and `.to_string()` when ownership is needed.
- `Next` internals are private; middleware continues through `Next::run`.
- Handler timeouts default to 504. `timeout_status` customizes the status, and
  timeout fallbacks receive the original method, URI, headers and extensions.
- The `jemalloc` feature re-exports `Jemalloc`; applications choose whether to
  declare it as their global allocator. SIMD parsers do not enable jemalloc.
- Signal identifiers and metadata keys use `Cow<'static, str>`; pass owned
  strings for dynamic identifiers. Request and route signals reach the router
  and app arbiters, with the matched route template and elapsed microseconds.
- `Router::state`, `set_state` and global GraphQL configuration are deprecated.
  Prefer router state or request-local GraphQL options.

### Added

- Fallible `try_spawn_*` server methods, `ServerHandle::result()` and
  `local_addr()`, plus a SIGINT/SIGTERM `shutdown_signal` helper and
  `ServerHandle::shutdown_on_signal()`.

- Owned `bytes::Bytes` and `String` body extractors; any `Responder` can be the
  error branch of a handler's `Result`.
- `error_handler_with_parts` exposes request metadata to error formatters;
  `Router::layer` supports chaining middleware with mutable builder methods.
- `#[head]` and `#[options]` shortcuts; umbrella forwarding for `queue-cron`,
  `socket-activation`, `vsock` and per-thread startup handles.


### Fixed

- HTTP/1 and HTTP/2 connections receive graceful shutdown notifications;
  Compio connection tasks remain owned until drained or cancelled. The explicit
  `ServerHandle::shutdown` deadline is enforced, including for stuck handlers.
- Routers and state are released when servers stop. Global metrics callbacks
  hold weak backend references, and TLS metadata clones avoid string allocation.
- Per-thread servers initialize plugins before accepting requests, install
  header timers, back off on accept errors, enforce connection limits and
  propagate worker-spawn failures. TLS handshake failures log at debug level.

- gRPC timeout parsing rejects non-ASCII units and invalid numeric prefixes
  without panicking. Parser fuzzing now covers ten input surfaces and runs
  weekly alongside stable/beta Clippy and dependency checks.

- Nested routes preserve scoped state, plugins, middleware, timeouts and body
  limits. Child state wins over parent state without leaking between siblings.
  Fallback and error formatting remain owned by the parent router.
- Problem JSON preserves semantic headers and invalidates representation metadata
  when replacing a body. Buffered 4xx text can supply the problem detail.
- Metrics count every completed route request without a lossy broadcast task,
  record latency and use router-local registries.
- Split SIMD features expose their corresponding extractor independently.
- Listener-free request signals skip payload construction; prefix subscriptions
  have a separate lookup and static metadata keys avoid string allocations.
- Workspace dependencies are inherited and ordered consistently; direct
  `once_cell` dependencies use standard-library lazy initialization instead.

- Trailing-slash redirects preserve the complete query string and retain
  their existing 307 status.
- HEAD requests fall back to matching GET routes unless an explicit HEAD
  route exists. Responses retain representation headers and omit the body,
  including responses produced by middleware and error handlers. `Allow`
  includes HEAD for GET routes.
- Repeated typed-state insertion and queue-handler registration replace the
  previous value as documented.
- Ready queue jobs drain without a polling delay between jobs. Handler panics
  move the job to dead letters without retrying it, and workers remain available.
- Concurrent plugin initialization waits until middleware installation completes.
- HTTP/2 cleartext and Tokio TLS servers provide a timer for configured keep-alive.
- GCRA rate limits accept the initial burst. Idle-key cleanup runs for both
  algorithms, less frequently, and retains slow quotas until their full burst
  could have refilled.
- Buffered compression skips SSE and bodies without an exact size. Both
  compression modes preserve partial responses and remove `Accept-Ranges`
  when transforming a complete representation.
- Buffered and streaming HTTP `deflate` responses include the required zlib
  wrapper so standard HTTP decoders can read them.
- Clippy 1.98 compatibility without changing public async or interceptor types;
  compression streams transfer their output buffers without copying them.

## [2.0.2] — 2026-07-20

### Added

- **Router introspection** — `Router::routes()` returns every registered route
  as `Vec<Arc<Route>>`, grouped by HTTP method and in registration order, with
  any `scope`/`nest` prefix already applied. Applications can derive
  reserved-path or namespace policies from the live route table instead of
  tracking registrations on the side.

## [2.0.1] — 2026-06-07

### Fixed

- Corrected feature propagation and publishing checks.
- Split large source modules and resolved Clippy warnings.
- Added the dedicated documentation site and corrected its content and styling.

## [2.0.0] — 2026-05-29

Tako 2.0 is the first long-term-stable release. It collapses every breaking
change that accumulated on `main` since 1.x into a single bump and makes
the workspace fully publishable to crates.io. The release also lands a
3-pass hardening audit (115 findings — 5 Critical, 18 High, 39 Medium,
53 Low — all closed) covering soundness, RFC compliance, lock-free hot
paths, and fail-closed defaults.

See [`MIGRATION_1_TO_2.md`](./MIGRATION_1_TO_2.md) for an upgrade walkthrough
covering every breaking change, including code-mod recipes for the macros and
typed-state APIs.

### Added

- **Per-router typed state** — `Router::with_state(T)` adds instance-local
  values alongside the process-global store; multiple routers can
  hold independent state of the same type.
- **Sub-routing primitives** — `Router::nest("/path", child)` and
  `Router::scope("/api", |s| { … })` register routes under a shared prefix;
  `Router::merge` remains available.
- **`Result`-aware handlers** — handlers may return `Result<R, E>` where
  `E: ResponderError`; `error_handler` is paired with a new `client_error_handler`,
  and `use_problem_json()` emits RFC 7807 `application/problem+json` bodies.
- **Method-aware routing** — non-matched verbs now return `405 Method Not
  Allowed` with the proper `Allow` header instead of `404`.
- **`Server::builder()`** — unified bootstrap across HTTP/1.1, HTTP/2,
  HTTP/3, TLS, mTLS, and Unix sockets, alongside the existing
  `serve_*` / `serve_tls_*` entry points.
- **TLS knobs** — `TlsCert::{Pem, Der, Resolver}`, `ReloadableResolver`,
  `ClientAuth` for full mTLS, SNI-based cert selection, and hot reload.
- **`ConnInfo`** — unified peer extension; replaces the `SocketAddr` /
  `UnixPeerAddr` split.
- **Runtime-agnostic `ServerHandle`** — graceful-shutdown handle that works
  uniformly across the Tokio and Compio runtimes.
- **Thread-per-core runtime** (`per-thread`, `per-thread-compio` features) —
  N×current-thread workers + `SO_REUSEPORT` bootstrap.

### Changed

- **MSRV: 1.95** — bumped from 1.87.
- **Edition: 2024** — workspace-wide.
- **Macros** — route paths support both `{id}` and `{id: u64}` forms; no
  `Params` struct is materialised unless a typed slot exists.
- **Workspace is fully publishable** — every internal sub-crate now
  publishes on crates.io as `tako-rs-core`, `tako-rs-extractors`,
  `tako-rs-macros`, `tako-rs-plugins`, `tako-rs-server`,
  `tako-rs-server-pt`, `tako-rs-streams` alongside the umbrella `tako-rs`
  crate. Use the umbrella crate; the sub-crates are considered
  implementation detail. (The unprefixed `tako-*` names are owned by an
  unrelated name-squatter at 0.0.0; the `tako-rs-*` prefix avoids that
  ownership conflict.)
- **`tako-core-local`** — the separate `!Send` router was removed; the
  unified `Router` is `Send + Sync` and serves both runtimes.
- **Compio runtime** — feature flags `compio`, `compio-tls`, `compio-ws`
  now compose cleanly with the rest of the framework. Compio is treated as
  a first-class runtime alongside Tokio.

### Removed

- **1.x `Params` global struct** — typed extractors replace it.

The `serve_*` free functions, `Router::merge`, and the process-global
`Router::state(T)` remain available. Prefer `Server::builder()` for server
configuration and `Router::with_state(T)` for instance-local state.

### Security

- **`cargo deny check`** is a CI gate; the v2 advisories schema fails the
  build on unignored vulnerabilities, unsoundness, and unmaintained crates.
- **mTLS support** via `ClientAuth` for hardened internal endpoints.

### Deferred to 2.x (not in this release)

The migration guide enumerates these in full; tracked separately from
breaking changes:

- `tako-stores-redis` / `tako-stores-postgres` companion crates (multi-replica
  SessionStore / RateLimitStore / IdempotencyStore backends).
- `TlsCert::Acme` (rustls-acme integration).
- HTTP/3 qlog (needs quinn bump).
- Multipart / byteranges responder + Linux `sendfile(2)` path on
  `FileStream`.
- Real WebTransport CONNECT handshake (currently aliased to raw-QUIC).
- gRPC reflection / health protobuf-generated stubs.
- Cluster `SignalBus` Redis / NATS / Kafka implementations.
- v2 client HTTP/2 + HTTP/3 + reqwest-style middleware.
- Hot-reload `Arc<Router>` swap.

## Releases before 2.0

Older 1.x release notes live on the
[GitHub releases page](https://github.com/rust-dd/tako/releases). The 1.x
line is in maintenance mode; bug-fix releases will continue if there is
user demand.

[Unreleased]: https://github.com/rust-dd/tako/compare/v2.0.2...HEAD
[2.0.2]: https://github.com/rust-dd/tako/compare/v2.0.1...v2.0.2
[2.0.1]: https://github.com/rust-dd/tako/compare/v2.0.0...v2.0.1
[2.0.0]: https://github.com/rust-dd/tako/releases/tag/v2.0.0
