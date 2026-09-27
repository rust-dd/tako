use std::sync::Arc;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use parking_lot::Mutex;
use scc::HashMap as SccHashMap;

use crate::stores::RateLimitDecision;
use crate::stores::RateLimitSnapshot;
use crate::stores::RateLimitStore;
use crate::stores::StoreResult;
#[derive(Clone)]
struct Bucket {
  available: f64,
  capacity: u32,
  refill_rate_per_sec: f64,
  last_refill: Instant,
}

impl Bucket {
  fn refill(&mut self, now: Instant) {
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
    assert!(
      capacity > 0 && refill_per_sec.is_finite() && refill_per_sec > 0.0,
      "capacity and refill rate must be positive and finite"
    );
    Self {
      capacity,
      refill_rate_per_sec: refill_per_sec,
      inner: Arc::new(SccHashMap::new()),
    }
  }
}

#[async_trait]
impl RateLimitStore for MemoryRateLimitStore {
  async fn consume(&self, key: &str, cost: u32) -> StoreResult<RateLimitDecision> {
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
    Ok(if allowed { Ok(snap) } else { Err(snap) })
  }
  async fn sweep(&self) -> StoreResult<()> {
    let retention =
      Duration::try_from_secs_f64((f64::from(self.capacity) / self.refill_rate_per_sec).max(300.0))
        .unwrap_or(Duration::MAX);
    self
      .inner
      .retain_async(|_, bucket| bucket.lock().last_refill.elapsed() < retention)
      .await;
    Ok(())
  }
}
