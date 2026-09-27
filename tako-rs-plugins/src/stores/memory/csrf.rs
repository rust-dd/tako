use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::time::Duration;
use std::time::Instant;

use async_trait::async_trait;
use scc::HashMap as SccHashMap;
use subtle::ConstantTimeEq;

use crate::stores::CsrfTokenStore;
use crate::stores::StoreResult;

struct Record {
  token: String,
  expires: Instant,
  used: AtomicBool,
}

/// In-memory session-bound CSRF tokens.
#[derive(Default, Clone)]
pub struct MemoryCsrfTokenStore {
  inner: Arc<SccHashMap<String, Record>>,
}

impl MemoryCsrfTokenStore {
  pub fn new() -> Self {
    Self::default()
  }
}

#[async_trait]
impl CsrfTokenStore for MemoryCsrfTokenStore {
  async fn issue(&self, session_id: &str, ttl: Duration) -> StoreResult<String> {
    let token = uuid::Uuid::new_v4().simple().to_string();
    self
      .inner
      .upsert_async(
        session_id.to_owned(),
        Record {
          token: token.clone(),
          expires: Instant::now() + ttl,
          used: AtomicBool::new(false),
        },
      )
      .await;
    Ok(token)
  }

  async fn validate(
    &self,
    session_id: &str,
    candidate: &str,
    single_use: bool,
  ) -> StoreResult<bool> {
    let Some(record) = self.inner.get_async(session_id).await else {
      return Ok(false);
    };
    if record.expires <= Instant::now()
      || !bool::from(record.token.as_bytes().ct_eq(candidate.as_bytes()))
      || record.used.load(Ordering::Acquire)
    {
      return Ok(false);
    }
    Ok(
      !single_use
        || record
          .used
          .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
          .is_ok(),
    )
  }

  async fn sweep(&self) -> StoreResult<()> {
    self
      .inner
      .retain_async(|_, record| {
        record.expires > Instant::now() && !record.used.load(Ordering::Acquire)
      })
      .await;
    Ok(())
  }
}
