use std::collections::VecDeque;

use super::QueueLimits;

pub(crate) struct Retention<T> {
  entries: VecDeque<(T, usize)>,
  bytes: usize,
  evicted: u64,
  max_jobs: usize,
  max_bytes: usize,
}

impl<T> Default for Retention<T> {
  fn default() -> Self {
    Self::new(QueueLimits::default())
  }
}

impl<T> Retention<T> {
  pub(crate) fn new(limits: QueueLimits) -> Self {
    Self {
      entries: VecDeque::new(),
      bytes: 0,
      evicted: 0,
      max_jobs: limits.dead_letters.unwrap_or(usize::MAX),
      max_bytes: limits.dead_letter_bytes.unwrap_or(usize::MAX),
    }
  }

  pub(crate) fn set_limits(&mut self, limits: QueueLimits) {
    self.max_jobs = limits.dead_letters.unwrap_or(usize::MAX);
    self.max_bytes = limits.dead_letter_bytes.unwrap_or(usize::MAX);
    while self.entries.len() > self.max_jobs || self.bytes > self.max_bytes {
      self.evict_oldest();
    }
  }

  pub(crate) fn push(&mut self, entry: T, bytes: usize) {
    if self.max_jobs == 0 || bytes > self.max_bytes {
      self.evicted = self.evicted.saturating_add(1);
      return;
    }
    while self.entries.len() >= self.max_jobs || bytes > self.max_bytes - self.bytes {
      self.evict_oldest();
    }
    self.bytes += bytes;
    self.entries.push_back((entry, bytes));
  }

  fn evict_oldest(&mut self) {
    if let Some((_, bytes)) = self.entries.pop_front() {
      self.bytes -= bytes;
      self.evicted = self.evicted.saturating_add(1);
    }
  }

  pub(crate) fn len(&self) -> usize {
    self.entries.len()
  }

  pub(crate) fn bytes(&self) -> usize {
    self.bytes
  }

  pub(crate) fn evictions(&self) -> u64 {
    self.evicted
  }

  pub(crate) fn iter(&self) -> impl Iterator<Item = &T> {
    self.entries.iter().map(|(entry, _)| entry)
  }

  pub(crate) fn clear(&mut self) {
    self.entries.clear();
    self.bytes = 0;
  }
}
