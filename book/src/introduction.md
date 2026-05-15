# Introduction

**Tako** is a multi-transport Rust framework for modern network
services. One application surface covers HTTP/1.1, HTTP/2, HTTP/3,
WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport — with
a shared routing, middleware, and observability model.

This handbook is the long-form companion to the in-source rustdoc.
The rustdoc is the reference; this book is the *guide*. When the two
disagree, the rustdoc wins for a single API; the book wins for the
intent and the recommended pattern.

## What is Tako?

The framework is built around five workspace crates, all re-exported
through the `tako-rs` umbrella under the `tako::*` path:

- `tako-core` — routing, handlers, middleware traits, body / request
  types, state, signals, queue, plus GraphQL, gRPC, and OpenAPI
  helpers.
- `tako-extractors` — concrete request extractors (cookies, form,
  query, path, JWT, multipart, simd-json, …).
- `tako-server` — HTTP/1.1, HTTP/2, HTTP/3, TLS, raw TCP / UDP / Unix,
  PROXY protocol, plus compio variants.
- `tako-streams` — WebSocket, SSE, file streaming, static file
  serving, WebTransport.
- `tako-plugins` — bundled middleware (auth, CSRF, sessions, …) and
  plugins (CORS, compression, rate limiting, idempotency, metrics).

Two ergonomic crates round out the public surface: `tako-macros` for
the `#[tako::route]` / `#[tako::get]` family and `tako-server-pt` for
the optional thread-per-core entry point.

The 2.0 release is the first cut where the routing, state, error,
and server-bootstrap stories are stable. See
[`reference/migration.md`](./reference/migration.md) for the full
list of breaking changes from 1.x.

## Who this is for

- **Service teams** building APIs that need more than plain REST —
  WebSockets, SSE, gRPC, raw TCP/UDP, or QUIC in the same binary as
  the HTTP routes.
- **Platform teams** that want a single Rust framework story across
  protocols, runtimes (Tokio + Compio), and deployment shapes
  (single binary, thread-per-core, sidecar over Unix sockets, behind
  PROXY-protocol load balancers).
- **Operators** who want first-class signals, queues, and graceful
  shutdown without composing them from scratch.

If you are building a plain JSON-over-HTTP service and never need
the realtime / multi-transport surface, you can use Tako as a
straight HTTP framework and ignore the rest of this book.

## How to read this book

- The **User guide** is task-oriented: pick the chapter for the thing
  you are building.
  - [Getting started](./guide/getting-started.md) covers installation
    and the first handler.
  - [Transports overview](./guide/transports.md) is the entry point
    for everything beyond HTTP/1.1.
  - [Routing](./guide/routing.md), [State](./guide/state.md),
    [Middleware](./guide/middleware.md), and
    [Extractors](./guide/extractors.md) walk through the request
    lifecycle.
  - [Streams](./guide/streams.md), [Queue](./guide/queue.md), and
    [Signals](./guide/signals.md) cover the framework primitives that
    aren't request-shaped.
  - [Observability](./guide/observability.md) and
    [Deployment](./guide/deployment.md) collect the production
    patterns.
- The **Reference** chapters are normative:
  - [API stability](./reference/stability.md) lists what is part of
    the semver contract and what is not.
  - [Migration: 1.x → 2.0](./reference/migration.md) is the
    breaking-change ledger.
  - [Cargo feature graph](./reference/features.md) explains each
    cargo feature.
  - [Runtime compatibility](./reference/runtimes.md) describes the
    tokio vs. compio split.

## Versions

This book ships with the framework. The rendered version lives at the
URL configured in `book.toml` and is rebuilt on every push to `main`.
For specific historical versions, browse the `book/` tree at the
release tag of interest. Code samples in this book are exercised
against the workspace as part of the 2.0 release process; if a sample
compiles in the book but not against the workspace, that is a CI bug
— please file it.
