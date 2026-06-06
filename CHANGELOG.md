# Changelog

All notable changes to **tako-rs** are documented here. Format inspired by
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); we follow
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

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

- **Per-router typed state** — `Router::with_state(T)` replaces the old
  per-type `GLOBAL_STATE` slot; multiple routers in the same process can now
  hold independent state of the same type.
- **Sub-routing primitives** — `Router::nest("/path", child)` and
  `Router::scope("/api", |s| { … })` replace `Router::merge` and add a real
  prefix-stripping pass.
- **`Result`-aware handlers** — handlers may return `Result<R, E>` where
  `E: Responder`; `error_handler` is paired with a new `client_error_handler`,
  and `use_problem_json()` emits RFC 7807 `application/problem+json` bodies.
- **Method-aware routing** — non-matched verbs now return `405 Method Not
  Allowed` with the proper `Allow` header instead of `404`.
- **`Server::builder()`** — unified bootstrap across HTTP/1.1, HTTP/2,
  HTTP/3, TLS, mTLS, and Unix sockets; replaces the matrix of
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

- **`serve_*` family of free functions** — use `Server::builder()`.
- **`Router::merge`** — use `nest` / `scope` instead.
- **`Router::state(T)` global slot** — use `Router::with_state(T)`.
- **1.x `Params` global struct** — typed extractors replace it.

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

[Unreleased]: https://github.com/rust-dd/tako/compare/v2.0.0...HEAD
[2.0.0]: https://github.com/rust-dd/tako/releases/tag/v2.0.0
