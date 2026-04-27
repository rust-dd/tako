#![cfg_attr(docsrs, feature(doc_cfg))]

//! Concrete plugin and middleware implementations for the Tako framework.
//!
//! The plugin and middleware traits (`TakoPlugin`, `IntoMiddleware`, `Next`)
//! live in `tako-core`. This crate hosts the concrete implementations:
//! built-in middleware (auth, CSRF, rate limiting, sessions, request IDs, ...)
//! and built-in plugins (CORS, compression, idempotency, metrics, rate
//! limiting). Re-exported under `tako::middleware::*` and `tako::plugins::*`
//! via the umbrella crate.

/// Concrete plugin implementations.
pub mod plugins;

/// Concrete middleware implementations.
pub mod middleware;
