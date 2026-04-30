#![cfg_attr(docsrs, feature(doc_cfg))]

//! `!Send` / thread-per-core core for the Tako framework.
//!
//! This crate mirrors the essentials of `tako-core` (`Handler`, `Next`, `Router`,
//! `Route`, `IntoMiddleware`) without the `Send + Sync` bounds. Handlers,
//! middleware, and route storage all use `Rc` instead of `Arc`, so handlers
//! may capture `!Send` state such as `Rc<RefCell<…>>` that lives entirely on
//! a single worker thread.
//!
//! Existing thread-safe handlers, middleware and extractors keep working:
//! blanket implementations make every `tako_core::handler::Handler<T>` a
//! `LocalHandler<T>` and every `tako_core::middleware::IntoMiddleware` a
//! `LocalIntoMiddleware`.
//!
//! Pair this with `tako-server-pt`'s `serve_per_thread_local` entry point.

pub mod handler;
pub mod middleware;
pub mod route;
pub mod router;
