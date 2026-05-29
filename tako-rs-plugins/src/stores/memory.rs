//! In-memory implementations of the [`super`] backend traits.
//!
//! These match the `scc::HashMap`-backed defaults that the built-in
//! middleware shipped with before the trait split. The trait split lets users
//! swap any of these out for Redis / Postgres / other shared backends without
//! forking the middleware itself.

use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;

use super::CsrfTokenStore;
use super::IdempotencyEntry;
use super::IdempotencyStore;
use super::JwksProvider;
use super::RateLimitSnapshot;
use super::RateLimitStore;
use super::SessionStore;

#[derive(Clone)]
struct SessionEntry {
  data: Vec<u8>,
  expires_at: Instant,
}

/// In-memory session backend.
#[derive(Default, Clone)]
pub struct MemorySessionStore {
  inner: Arc<SccHashMap<String, SessionEntry>>,
}

impl MemorySessionStore {
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl SessionStore for MemorySessionStore {
  async fn load(&self, id: &str) -> Option<Vec<u8>> {
    let entry = self.inner.get_async(id).await?;
    if entry.expires_at <= Instant::now() {
      return None;
    }
    Some(entry.data.clone())
  }

  async fn store(&self, id: &str, data: Vec<u8>, ttl: Duration) {
    let entry = SessionEntry {
      data,
      expires_at: Instant::now() + ttl,
    };
    let _ = self.inner.upsert_async(id.to_string(), entry).await;
  }

  async fn remove(&self, id: &str) -> bool {
    self.inner.remove_async(id).await.is_some()
  }

  async fn sweep(&self) {
    let now = Instant::now();
    self.inner.retain_async(|_, v| v.expires_at > now).await;
  }
}

#[derive(Clone)]
struct Bucket {
  available: f64,
  capacity: u32,
  refill_rate_per_sec: f64,
  last_refill: Instant,
}

impl Bucket {
  fn refill(&mut self, now: Instant) {
    // `Instant::duration_since` saturates at zero for past-or-equal points,
    // and `Instant` is monotonic on every platform we ship — `dt` cannot go
    // negative. The previous `if dt > 0.0` branch was therefore dead in
    // release; the only practical hit was `dt == 0.0` on the same-tick
    // double-refill case, which a `debug_assert!` makes visible without
    // costing a branch in release.
    let dt = now.duration_since(self.last_refill).as_secs_f64();
    debug_assert!(dt >= 0.0, "monotonic Instant violated: dt={dt}");
    self.available = (self.available + dt * self.refill_rate_per_sec).min(f64::from(self.capacity));
    self.last_refill = now;
  }
}

/// Token-bucket in-memory rate limiter.
#[derive(Clone)]
pub struct MemoryRateLimitStore {
  capacity: u32,
  refill_rate_per_sec: f64,
  inner: Arc<SccHashMap<String, Arc<Mutex<Bucket>>>>,
}

impl MemoryRateLimitStore {
  /// `capacity` is the burst size; `refill_per_sec` adds tokens continuously.
  pub fn new(capacity: u32, refill_per_sec: f64) -> Self {
    Self {
      capacity,
      refill_rate_per_sec: refill_per_sec,
      inner: Arc::new(SccHashMap::new()),
    }
  }
}

#[async_trait]
impl RateLimitStore for MemoryRateLimitStore {
  async fn consume(&self, key: &str, cost: u32) -> Result<RateLimitSnapshot, RateLimitSnapshot> {
    let capacity = self.capacity;
    let refill_rate = self.refill_rate_per_sec;
    let mutex = {
      let entry = self
        .inner
        .entry_async(key.to_string())
        .await
        .or_insert_with(|| {
          Arc::new(Mutex::new(Bucket {
            available: f64::from(capacity),
            capacity,
            refill_rate_per_sec: refill_rate,
            last_refill: Instant::now(),
          }))
        });
      entry.get().clone()
    };
    let mut bucket = mutex.lock();
    let now = Instant::now();
    bucket.refill(now);
    let cost_f = f64::from(cost);
    let allowed = bucket.available >= cost_f;
    if allowed {
      bucket.available -= cost_f;
    }
    let remaining = bucket.available.max(0.0).floor() as u32;
    let needed = (cost_f - bucket.available).max(0.0);
    let reset_secs = if bucket.refill_rate_per_sec > 0.0 {
      (needed / bucket.refill_rate_per_sec).ceil() as u64
    } else {
      0
    };
    let retry_after_secs = if allowed { 0 } else { reset_secs.max(1) };
    let snap = RateLimitSnapshot {
      limit: bucket.capacity,
      remaining,
      reset_secs,
      retry_after_secs,
    };
    if allowed { Ok(snap) } else { Err(snap) }
  }
}

#[derive(Clone)]
struct StoredIdempotency {
  entry: IdempotencyEntry,
  expires_at: Instant,
}

/// In-memory idempotency cache.
#[derive(Clone)]
pub struct MemoryIdempotencyStore {
  inner: Arc<SccHashMap<String, StoredIdempotency>>,
  inflight_ttl: Duration,
}

impl Default for MemoryIdempotencyStore {
  fn default() -> Self {
    Self {
      inner: Arc::new(SccHashMap::new()),
      // Long enough to outlive a typical synchronous handler but short
      // enough to drop crashed in-flight entries before they leak. Override
      // with `with_inflight_ttl` for slow uploads / long-running handlers.
      inflight_ttl: Duration::from_secs(300),
    }
  }
}

impl MemoryIdempotencyStore {
  pub fn new() -> Self {
    Self::default()
  }

  /// Override the TTL used for in-flight `begin` entries. Default 300s.
  /// Set this to be at least as long as the slowest handler that may
  /// register an idempotency key — anything shorter risks an in-flight
  /// entry expiring before the handler completes, breaking coalescing.
  pub fn with_inflight_ttl(mut self, ttl: Duration) -> Self {
    self.inflight_ttl = ttl;
    self
  }
}

#[async_trait]
impl IdempotencyStore for MemoryIdempotencyStore {
  async fn get(&self, key: &str) -> Option<IdempotencyEntry> {
    let stored = self.inner.get_async(key).await?;
    if stored.expires_at <= Instant::now() {
      return None;
    }
    Some(stored.entry.clone())
  }

  async fn begin(&self, key: &str, payload_sig: [u8; 20]) -> IdempotencyEntry {
    let entry = IdempotencyEntry {
      status: 0,
      headers: Vec::new(),
      body: Vec::new(),
      payload_sig,
      completed: false,
    };
    let stored = StoredIdempotency {
      entry: entry.clone(),
      expires_at: Instant::now() + self.inflight_ttl,
    };
    let _ = self.inner.upsert_async(key.to_string(), stored).await;
    entry
  }

  async fn complete(&self, key: &str, entry: IdempotencyEntry, ttl: Duration) {
    let stored = StoredIdempotency {
      entry,
      expires_at: Instant::now() + ttl,
    };
    let _ = self.inner.upsert_async(key.to_string(), stored).await;
  }

  async fn remove(&self, key: &str) {
    let _ = self.inner.remove_async(key).await;
  }
}

/// Static-snapshot JWKS provider.
#[derive(Default, Clone)]
pub struct StaticJwksProvider {
  by_kid: Arc<SccHashMap<String, Vec<Vec<u8>>>>,
}

impl StaticJwksProvider {
  pub fn new() -> Self {
    Self::default()
  }

  /// Adds a key under `kid`. Multiple keys per kid are supported (rotation).
  ///
  /// PPL-25: the previous `update_sync` + `insert_sync` fallback was racy.
  /// Two threads both observing `update_sync == None` would each call
  /// `insert_sync`; only the first wins, the loser's key was silently
  /// dropped — a rotation footgun where the new key would not be findable
  /// at verify time. `entry_sync(...).and_modify(...).or_insert_with(...)`
  /// performs the lookup-or-insert atomically under the scc bucket lock.
  pub fn insert(&self, kid: impl Into<String>, key: Vec<u8>) {
    let kid = kid.into();
    self
      .by_kid
      .entry_sync(kid)
      .and_modify(|v| v.push(key.clone()))
      .or_insert_with(|| vec![key]);
  }
}

#[async_trait]
impl JwksProvider for StaticJwksProvider {
  async fn keys_for(&self, kid: &str) -> Vec<Vec<u8>> {
    self
      .by_kid
      .get_async(kid)
      .await
      .map(|v| v.clone())
      .unwrap_or_default()
  }
}

#[derive(Clone)]
struct CsrfRecord {
  token: String,
  expires_at: Instant,
  uses_left: Arc<AtomicU32>,
}

/// In-memory CSRF token store.
#[derive(Default, Clone)]
pub struct MemoryCsrfTokenStore {
  inner: Arc<SccHashMap<String, CsrfRecord>>,
}

impl MemoryCsrfTokenStore {
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl CsrfTokenStore for MemoryCsrfTokenStore {
  async fn issue(&self, session_id: &str, ttl: Duration) -> String {
    let token = uuid::Uuid::new_v4().simple().to_string();
    let record = CsrfRecord {
      token: token.clone(),
      expires_at: Instant::now() + ttl,
      uses_left: Arc::new(AtomicU32::new(u32::MAX)),
    };
    let _ = self
      .inner
      .upsert_async(session_id.to_string(), record)
      .await;
    token
  }

  async fn validate(&self, session_id: &str, candidate: &str, single_use: bool) -> bool {
    let record = self.inner.get_async(session_id).await;
    let Some(record) = record else {
      return false;
    };
    if record.expires_at <= Instant::now() {
      return false;
    }
    if record.token != candidate {
      return false;
    }
    if single_use {
      // CAS decrement: `fetch_sub(1)` on a zero counter underflows to
      // `u32::MAX`, which would silently re-arm the credential. Loop with
      // `compare_exchange` so we only consume an actually-positive count.
      loop {
        let cur = record.uses_left.load(Ordering::Acquire);
        if cur == 0 {
          return false;
        }
        if record
          .uses_left
          .compare_exchange(cur, cur - 1, Ordering::AcqRel, Ordering::Acquire)
          .is_ok()
        {
          break;
        }
      }
    }
    true
  }
}
