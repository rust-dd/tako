//! Pluggable backend traits for stateful middleware.
//!
//! Built-in middleware (sessions, rate limiting, idempotency, JWKS, CSRF) all
//! ship with an in-memory `scc::HashMap` store. Production deployments often
//! want to swap that out for Redis, Postgres, or another shared backend so a
//! cluster of replicas can share state. The traits here define the minimum
//! surface needed by each middleware.
//!
//! Concrete `Memory*` implementations live in submodules under this module.
//! Crates that want to provide a Redis or Postgres backend can implement the
//! traits in their own crate and pass the resulting type into the matching
//! middleware builder.
//!
//! # TODO — Redis / Postgres backend crates (tracked for v2.0)
//!
//! Companion crates `tako-stores-redis` and `tako-stores-postgres` are
//! planned but **not yet shipped**. Until they land, multi-replica
//! deployments must implement these traits themselves (or accept the
//! per-process state silos of the in-memory defaults). See `V2_ROADMAP.md`
//! § 4.1 for the linked follow-up checklist — do not let this slip.

use std::time::Duration;

use async_trait::async_trait;

pub mod memory;

/// Persistent session storage.
///
/// Implementations must be safe to clone cheaply — sessions are accessed on
/// every request, so the trait is invoked from inside hot middleware paths.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
  /// Reads a session blob keyed by `id`. Returns `None` if the session does
  /// not exist or has expired.
  async fn load(&self, id: &str) -> Option<Vec<u8>>;

  /// Inserts or replaces the session blob for `id` with the configured TTL.
  async fn store(&self, id: &str, data: Vec<u8>, ttl: Duration);

  /// Removes the session, returning whether the key existed.
  async fn remove(&self, id: &str) -> bool;

  /// Optional sweep hook. The default in-memory store schedules its own
  /// janitor; remote backends typically rely on TTL expiry inside the
  /// underlying database (e.g. Redis `EXPIRE`).
  async fn sweep(&self) {}
}

/// Token-bucket / GCRA rate-limit storage.
///
/// `consume` atomically reduces the bucket for `key` by one request and
/// returns the post-consumption snapshot. Implementations are responsible for
/// refilling the bucket — token-bucket tickers run on a per-store schedule,
/// GCRA computes the new state on read.
#[async_trait]
pub trait RateLimitStore: Send + Sync + 'static {
  /// Atomically attempts to take one permit from `key`'s bucket. Returns
  /// `Ok(snapshot)` when the request is allowed, `Err(snapshot)` when the
  /// caller exceeded the limit. The returned snapshot is what the caller
  /// emits in the `RateLimit-*` response headers.
  async fn consume(&self, key: &str, cost: u32) -> Result<RateLimitSnapshot, RateLimitSnapshot>;
}

/// Public snapshot of a rate-limit decision suitable for response headers.
#[derive(Debug, Clone)]
pub struct RateLimitSnapshot {
  /// Configured maximum (`RateLimit-Limit` value).
  pub limit: u32,
  /// Remaining quota after the current request, never below zero.
  pub remaining: u32,
  /// Seconds until the next refill arrives (`RateLimit-Reset`).
  pub reset_secs: u64,
  /// Suggested `Retry-After` (only meaningful when the request was rejected).
  pub retry_after_secs: u64,
}

/// Idempotency-key cache.
#[async_trait]
pub trait IdempotencyStore: Send + Sync + 'static {
  /// Reads an existing entry for `key`.
  async fn get(&self, key: &str) -> Option<IdempotencyEntry>;

  /// Marks `key` as in-flight; returns the freshly inserted record, or the
  /// existing one if another request arrived first.
  async fn begin(&self, key: &str, payload_sig: [u8; 20]) -> IdempotencyEntry;

  /// Persists a completed entry with the configured TTL.
  async fn complete(&self, key: &str, entry: IdempotencyEntry, ttl: Duration);

  /// Removes the entry — typically invoked when the handler decided not to
  /// cache the result (e.g. opt-out via response header).
  async fn remove(&self, key: &str);
}

/// Idempotency cache record. The body / headers are stored as opaque bytes so
/// remote backends don't need to understand HTTP serialization.
#[derive(Debug, Clone)]
pub struct IdempotencyEntry {
  pub status: u16,
  pub headers: Vec<(String, Vec<u8>)>,
  pub body: Vec<u8>,
  pub payload_sig: [u8; 20],
  pub completed: bool,
}

/// JSON Web Key Set provider.
///
/// `keys_for(kid)` returns the candidate verification keys for a given key
/// id. JWKS rotation is implementation-specific: the in-memory provider
/// caches a fixed snapshot, while remote providers typically fetch from a
/// well-known URL with their own background refresh cadence.
#[async_trait]
pub trait JwksProvider: Send + Sync + 'static {
  /// Returns matching key bytes for `kid`. Multiple matches are allowed
  /// (handlers verify against each in order); `None` means "no rotation
  /// match — fall back to the configured default key, if any".
  async fn keys_for(&self, kid: &str) -> Vec<Vec<u8>>;
}

/// CSRF token storage. Used by token-store CSRF middleware (as opposed to the
/// stateless double-submit-cookie variant).
#[async_trait]
pub trait CsrfTokenStore: Send + Sync + 'static {
  /// Issues a token bound to the given session id with the configured TTL.
  async fn issue(&self, session_id: &str, ttl: Duration) -> String;

  /// Validates a candidate token against the session id, consuming it on
  /// success when `single_use` is true.
  async fn validate(&self, session_id: &str, token: &str, single_use: bool) -> bool;
}
