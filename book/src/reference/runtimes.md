# Runtime compatibility

Tako runs on two async runtimes:

- **Tokio** (the default). Multi-threaded scheduler, work-stealing,
  `Send` futures end-to-end. This is what the standalone `Server`
  builder targets.
- **Compio** (opt-in via `compio` feature). Single-threaded
  thread-per-core. Futures are `!Send` and pinned to the runtime
  thread that produced them.

## You cannot enable both at once

`cargo build --all-features` does not compile. The reason:

- Hyper's HTTP/2 service bound is `+ Send`.
- `compio::time::sleep` is `!Send`.
- Some middleware (notably `tako_plugins::middleware::timeout::Timeout`)
  has to choose one runtime per build via `#[cfg(...)]`.

Pick **one** runtime per binary. If a single deployment needs both,
build separate binaries.

## `send_wrapper` invariant

Where the framework does cross the runtime boundary internally — for
example, hyper HTTP/2 over compio TLS — `send_wrapper::SendWrapper` is
used to satisfy hyper's `Send` bound at the type level. The wrapper
panics on cross-thread access at runtime, so a misuse becomes a loud
panic, not UB.

The soundness contract is **per-runtime, not global**: every wrapped
future is constructed and polled on the same compio runtime thread,
and never handed back to a multi-threaded tokio executor. See the
module-level rustdoc on
`tako_server::server_tls_compio` for the full invariant statement.

## Per-thread runtimes

`per-thread` and `per-thread-compio` host the same thread-safe
`Router` on N current-thread workers (`SO_REUSEPORT`-fanned).
`spawn_per_thread` returns a shutdown handle that drives a `select!`
over each worker's accept loop so termination is clean.
