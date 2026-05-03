# Deployment

> **Status:** scaffold.

## Single binary

The default. `Server::builder` + tokio multi-threaded runtime. Use
`max_connections` to bound accept-spawn pressure and `drain_timeout`
to control graceful shutdown.

## Thread-per-core

Two flavors, both behind cargo features:

- `per-thread` — `tokio` `current_thread` runtime per core, sharing
  the same thread-safe `Router` via `Box::leak`. Backed by
  `SO_REUSEPORT` for kernel-level fan-out.
- `per-thread-compio` — io_uring (Linux) / IOCP (Windows) / kqueue
  (macOS) via `compio`. Same per-worker shape.

`spawn_per_thread` returns `(Vec<JoinHandle>, PerThreadShutdown)`.
The shutdown handle drives a `select!` over the accept loop so
workers exit cleanly on signal.

## Behind a load balancer

- `Server::spawn_proxy_protocol` consumes PROXY v1 / v2 (TLV-aware,
  CRC32C-verified) and rewrites `X-Forwarded-*` from the parsed
  header.
- `tako_server::server_h2c` for L7 HTTP/2 proxies (Envoy, Nginx) that
  terminate TLS and forward cleartext h2c upstream.

## Socket activation

Behind `socket-activation` cargo feature: `LISTEN_FDS` /
`LISTEN_PID` (and the s6 / catflap equivalents) are read by
`tako_server::socket_activation::ListenFds::from_env()`. Returned
listener types feed the existing `Server::spawn_*` methods.

## Hot-reload

The default path keeps `&'static Router` for hot-path performance.
Hot-reload via `arc_swap::ArcSwap<Arc<Router>>` is on the v2
follow-up list; once it lands, opt in via a single `Server::builder`
flag.
