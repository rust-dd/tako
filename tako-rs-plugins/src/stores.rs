//! Async backends for stateful middleware.
//!
//! Builders accept these traits through `.store(...)`; memory implementations
//! are available under [`memory`]. Remote stores must implement atomic quota
//! consumption and idempotency leases in the shared backend itself.

use std::time::Duration;

use async_trait::async_trait;

pub mod memory;
pub(crate) mod runtime;

/// A backend failure. Middleware logs the cause and fails closed.
pub type StoreResult<T> = Result<T, tako_rs_core::types::BoxError>;

/// Opaque session blobs with expiration enforced on reads.
#[async_trait]
pub trait SessionStore: Send + Sync + 'static {
  async fn load(&self, id: &str) -> StoreResult<Option<Vec<u8>>>;
  async fn store(&self, id: &str, data: Vec<u8>, ttl: Duration) -> StoreResult<()>;
  async fn remove(&self, id: &str) -> StoreResult<bool>;
  /// Optional cleanup for expired entries; remote stores can use database TTLs.
  async fn sweep(&self) -> StoreResult<()> {
    Ok(())
  }
}

/// A quota decision: allowed or rejected, each with its response metadata.
pub type RateLimitDecision = Result<RateLimitSnapshot, RateLimitSnapshot>;

/// Atomically consume quota. The backend owns capacity, refill, and expiry policy.
#[async_trait]
pub trait RateLimitStore: Send + Sync + 'static {
  async fn consume(&self, key: &str, cost: u32) -> StoreResult<RateLimitDecision>;
  async fn sweep(&self) -> StoreResult<()> {
    Ok(())
  }
}

/// Response metadata returned by a rate-limit backend.
#[derive(Debug, Clone)]
pub struct RateLimitSnapshot {
  pub limit: u32,
  pub remaining: u32,
  pub reset_secs: u64,
  pub retry_after_secs: u64,
}

/// The result of atomically acquiring an idempotency key.
#[derive(Debug, Clone)]
pub enum IdempotencyBegin {
  /// An opaque lease token owned by this request.
  Acquired(String),
  /// A live pending or completed entry owned by another request.
  Existing(IdempotencyEntry),
}

/// Distributed idempotency leases and cached responses.
///
/// Lease expiry must exceed the application's maximum handler duration.
/// Completion/removal must compare the lease token atomically, so an expired
/// owner cannot overwrite or delete a newer request's result.
#[async_trait]
pub trait IdempotencyStore: Send + Sync + 'static {
  async fn get(&self, key: &str) -> StoreResult<Option<IdempotencyEntry>>;
  async fn begin(&self, key: &str, payload_sig: [u8; 32]) -> StoreResult<IdempotencyBegin>;
  async fn complete(
    &self,
    key: &str,
    lease: &str,
    entry: IdempotencyEntry,
    ttl: Duration,
  ) -> StoreResult<bool>;
  async fn remove(&self, key: &str, lease: &str) -> StoreResult<bool>;
  /// Wait for another owner. Remote stores can override polling with pub/sub.
  async fn wait(
    &self,
    key: &str,
    timeout: Option<Duration>,
  ) -> StoreResult<Option<IdempotencyEntry>> {
    let started = std::time::Instant::now();
    loop {
      let entry = self.get(key).await?;
      if entry.as_ref().is_none_or(|entry| entry.completed) {
        return Ok(entry);
      }
      let delay = timeout.map_or(Duration::from_millis(20), |limit| {
        limit
          .saturating_sub(started.elapsed())
          .min(Duration::from_millis(20))
      });
      if delay.is_zero() {
        return Ok(entry);
      }
      runtime::sleep(delay).await;
    }
  }
  async fn sweep(&self) -> StoreResult<()> {
    Ok(())
  }
}

/// An idempotency record with serializable response bytes.
#[derive(Debug, Clone)]
pub struct IdempotencyEntry {
  pub status: u16,
  pub headers: Vec<(String, Vec<u8>)>,
  pub body: bytes::Bytes,
  pub payload_sig: [u8; 32],
  pub completed: bool,
}

/// A verification key bound to one JWT algorithm, preventing cross-algorithm reuse.
#[derive(Debug, Clone)]
pub struct VerificationKey {
  pub algorithm: String,
  /// Raw MAC bytes or DER public-key bytes for the bundled verifier.
  pub bytes: Vec<u8>,
}

/// Candidate verification keys for a JWT `kid`.
///
/// Key encoding is defined by the verifier. An empty list uses its configured
/// static keys; provider errors and failed candidate signatures fail closed.
#[async_trait]
pub trait JwksProvider: Send + Sync + 'static {
  async fn keys_for(&self, kid: &str) -> StoreResult<Vec<VerificationKey>>;
}

/// Tokens bound to a session identity; single-use validation must be atomic.
#[async_trait]
pub trait CsrfTokenStore: Send + Sync + 'static {
  async fn issue(&self, session_id: &str, ttl: Duration) -> StoreResult<String>;
  async fn validate(&self, session_id: &str, token: &str, single_use: bool) -> StoreResult<bool>;
  async fn sweep(&self) -> StoreResult<()> {
    Ok(())
  }
}
