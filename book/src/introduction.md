# Introduction

**Tako** is a multi-transport Rust framework for modern network
services. One application surface covers HTTP/1.1, HTTP/2, HTTP/3,
WebSocket, SSE, gRPC, TCP, UDP, Unix sockets, and WebTransport — with
a shared routing, middleware, and observability model.

This handbook is the long-form companion to the in-source rustdoc. The
rustdoc is the reference; this book is the *guide*. When the two
disagree, the rustdoc wins for a single API; the book wins for the
intent and the recommended pattern.

## Who this is for

- Service teams building APIs that need more than plain REST.
- Platform teams that want a single Rust framework story across
  protocols, runtimes (Tokio + Compio), and deployment shapes.
- Operators who want first-class signals, queue, and graceful
  shutdown without composing them from scratch.

## How to read this book

- The **User guide** is task-oriented: pick the chapter for the thing
  you are building.
- The **Reference** chapters are normative: API stability, feature
  graph, MSRV, migration path.
- Code samples in this book are tested in CI via `mdbook test`. If a
  sample compiles in the book but not against the workspace, that is
  a CI bug — please file it.

## Versions

This book ships with the framework. The rendered version lives at the
URL configured in `book.toml` and is rebuilt on every push to `main`.
For specific historical versions, browse the `book/` tree at the
release tag of interest.
