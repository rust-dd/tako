#![cfg(feature = "tls")]
#![cfg_attr(docsrs, doc(cfg(feature = "tls")))]

//! TLS-enabled HTTP server implementation for secure connections (compio runtime).
//!
//! # `send_wrapper` invariant — hard contract
//!
//! Hyper's HTTP/2 server builder requires `Send` on the response future and
//! the executor it hands work to. The compio runtime is **single-threaded
//! per core**: every future created by `compio::runtime::spawn` is `!Send`
//! and is polled exclusively on the runtime thread that produced it.
//!
//! Reconciling these two facts is the entire reason `send_wrapper` shows up
//! in this file:
//!
//! * `ServiceSendWrapper` wraps the per-connection hyper service and its
//!   response future in `SendWrapper`, satisfying hyper's bound at the type
//!   level.
//! * `CompioH2Executor` re-`spawn`s those `Send`-claimed futures back onto
//!   the same compio runtime thread.
//! * `CompioH2Timer` wraps `compio::time::sleep` similarly so HTTP/2
//!   keep-alive timers can be handed to hyper.
//!
//! **The soundness of this pattern depends on the wrapped values never
//! crossing a thread boundary at runtime.** That holds because:
//!
//! 1. The compio runtime is per-thread — futures are pinned to the thread
//!    that called `spawn`, and there is no cross-thread work-stealing.
//! 2. `SendWrapper<T>` panics on drop or deref from any thread other than
//!    the one that constructed it, so an accidental cross-thread move
//!    becomes a loud panic instead of UB.
//! 3. We never construct a `SendWrapper` outside of a compio runtime task,
//!    and we never hand the wrapper to a multi-threaded tokio runtime.
//!
//! The `Send` claim made by `SendWrapper<T>` is therefore **per-runtime, not
//! global**. Anyone moving these types out of the compio path (e.g. mixing
//! a tokio executor in front of `ServiceSendWrapper`) breaks the invariant.

mod accept;
mod executor;
mod serve;

pub use accept::run_with_config;
pub use serve::load_certs;
pub use serve::load_key;
pub use serve::run;
pub use serve::serve_tls;
pub use serve::serve_tls_with_config;
pub use serve::serve_tls_with_rustls_config;
pub use serve::serve_tls_with_rustls_config_and_shutdown;
pub use serve::serve_tls_with_shutdown;
pub use serve::serve_tls_with_shutdown_and_config;
