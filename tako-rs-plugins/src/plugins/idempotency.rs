#![cfg_attr(docsrs, doc(cfg(feature = "plugins")))]
//! Idempotency-Key based request de-duplication plugin.
//!
//! This plugin implements server-side idempotency for unsafe methods (typically POST),
//! keyed by a caller-provided header (default: `Idempotency-Key`). For a given key and
//! scope, it ensures that concurrent or repeated requests return the exact same response
//! (status, selected headers, body) within a configurable TTL.
//!
//! Behavior:
//! - First request with a new key is processed normally while marking the key as in-flight.
//! - Concurrent requests with the same key wait for completion and receive the cached result.
//! - Replays within TTL return the cached result immediately.
//! - If the same key is reused with a different payload, a 409 Conflict is returned.
//!
//! Notes:
//! - Bodies are buffered to compute a stable payload signature and to cache responses.
//! - Response headers are filtered to exclude hop-by-hop and length-specific headers.
//! - Storage is in-memory; TTL-based cleanup runs periodically.

mod config;
mod plugin;
mod response;
mod store;

pub use config::Config;
pub use config::IdempotencyBuilder;
pub use config::Scope;
pub use plugin::IdempotencyPlugin;
