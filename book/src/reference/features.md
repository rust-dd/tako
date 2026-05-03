# Cargo feature graph

> **Status:** scaffold — generated programmatically as part of CI in
> a follow-up.

The umbrella crate `tako-rs` is the contractual surface; sub-crate
features are reached through it. Run

```bash
cargo metadata --format-version 1 --no-deps \
  | jq '.packages[] | select(.name == "tako-rs") | .features'
```

to see the current shape locally. High-traffic features:

- `tls` / `http2` / `http3` — transport feature flags.
- `compio` / `compio-tls` / `compio-ws` — the alternative runtime
  surface.
- `plugins` / `signals` — middleware and observability primitives.
- `multipart` / `simd` / `protobuf` / `typed-header` /
  `zero-copy-extractors` — extractor add-ons.
- `metrics-prometheus` / `metrics-opentelemetry` / `tako-tracing` —
  observability backends.
- `validator` / `garde` — request validation crates.
- `client` (default-off) — outbound HTTP client. Pair with
  `native-certs` to use the OS trust store instead of the bundled
  `webpki-roots` snapshot.
- `per-thread` / `per-thread-compio` — thread-per-core deployment.
- `ip-filter`, `hmac-signature`, `json-schema`, `zstd` — opt-in
  middleware.
- `jwt-simple`, `ahash`, `jemalloc` — algorithm / hashing /
  allocator opt-ins.

> Enabling **both** the tokio and compio runtime sides at once is
> not supported; see [Runtime compatibility](./runtimes.md).
