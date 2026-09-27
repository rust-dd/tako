use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::collections::HashMap;
use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Instant;

pub(crate) struct PendingJob {
  pub(crate) id: u64,
  pub(crate) name: String,
  pub(crate) payload: Vec<u8>,
  pub(crate) attempt: u32,
  pub(crate) run_after: Option<Instant>,
  pub(crate) dedup_key: Option<Arc<str>>,
}

struct ScheduledJob {
  deadline: Instant,
  job: PendingJob,
}

impl PartialEq for ScheduledJob {
  fn eq(&self, other: &Self) -> bool {
    (self.deadline, self.job.id) == (other.deadline, other.job.id)
  }
}
impl Eq for ScheduledJob {}

impl PartialOrd for ScheduledJob {
  fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
    Some(self.cmp(other))
  }
}

impl Ord for ScheduledJob {
  fn cmp(&self, other: &Self) -> Ordering {
    (other.deadline, other.job.id).cmp(&(self.deadline, self.job.id))
  }
}

#[derive(Default)]
pub(crate) struct PendingQueue {
  ready: VecDeque<PendingJob>,
  delayed: BinaryHeap<ScheduledJob>,
  // Retries can overlap a newer job whose dedup window started after the claim.
  dedup: HashMap<Arc<str>, smallvec::SmallVec<[u64; 1]>>,
}

impl PendingQueue {
  pub(crate) fn len(&self) -> usize {
    self.ready.len() + self.delayed.len()
  }

  pub(crate) fn is_empty(&self) -> bool {
    self.ready.is_empty() && self.delayed.is_empty()
  }

  pub(crate) fn existing_id(&self, key: &str) -> Option<u64> {
    self.dedup.get(key).and_then(|ids| ids.first().copied())
  }

  pub(crate) fn push(&mut self, job: PendingJob) {
    if let Some(key) = &job.dedup_key {
      self.dedup.entry(key.clone()).or_default().push(job.id);
    }
    if let Some(deadline) = job.run_after {
      self.delayed.push(ScheduledJob { deadline, job });
    } else {
      self.ready.push_back(job);
    }
  }

  pub(crate) fn pop_ready(&mut self, now: Instant) -> Option<PendingJob> {
    while self.delayed.peek().is_some_and(|job| job.deadline <= now) {
      self.ready.push_back(self.delayed.pop().unwrap().job);
    }
    let job = self.ready.pop_front()?;
    if let Some(key) = &job.dedup_key {
      let remove = if let Some(ids) = self.dedup.get_mut(key) {
        ids.retain(|id| *id != job.id);
        ids.is_empty()
      } else {
        false
      };
      if remove {
        self.dedup.remove(key);
      }
    }
    Some(job)
  }

  pub(crate) fn next_deadline(&self) -> Option<Instant> {
    self.delayed.peek().map(|job| job.deadline)
  }

  pub(crate) fn drain(&mut self) -> Vec<PendingJob> {
    self.dedup.clear();
    self
      .ready
      .drain(..)
      .chain(self.delayed.drain().map(|scheduled| scheduled.job))
      .collect()
  }
}

#[cfg(test)]
mod tests {
  use std::time::Duration;

  use super::*;

  fn job(id: u64, deadline: Option<Instant>, key: Option<&str>) -> PendingJob {
    PendingJob {
      id,
      name: "test".into(),
      payload: vec![],
      attempt: 0,
      run_after: deadline,
      dedup_key: key.map(Into::into),
    }
  }

  #[test]
  fn delayed_order_and_overlapping_retry_keys_are_preserved() {
    let now = Instant::now();
    let mut queue = PendingQueue::default();
    queue.push(job(1, Some(now + Duration::from_secs(20)), Some("same")));
    queue.push(job(2, None, Some("same")));
    queue.push(job(3, Some(now + Duration::from_secs(10)), None));
    assert_eq!(queue.pop_ready(now).unwrap().id, 2);
    assert_eq!(queue.existing_id("same"), Some(1));
    assert_eq!(queue.next_deadline(), Some(now + Duration::from_secs(10)));
    assert_eq!(
      queue.pop_ready(now + Duration::from_secs(15)).unwrap().id,
      3
    );
    assert!(queue.pop_ready(now + Duration::from_secs(15)).is_none());
    assert_eq!(
      queue.pop_ready(now + Duration::from_secs(20)).unwrap().id,
      1
    );
    assert_eq!(queue.existing_id("same"), None);
    assert!(queue.is_empty());
  }
}
