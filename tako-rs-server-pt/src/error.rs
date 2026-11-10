use std::any::Any;
use std::fmt;
use std::io;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum PerThreadErrorKind {
  /// No listener could bind. Individual bind failures may be tolerated.
  Bind,
  /// Runtime initialization or plugin setup failed.
  Startup,
  Io,
  Panic,
  /// Returned without a shutdown request.
  UnexpectedExit,
}

/// Worker failure reported by [`PerThreadHandle`](crate::PerThreadHandle).
#[derive(Clone, Debug)]
pub struct PerThreadError {
  worker_id: usize,
  kind: PerThreadErrorKind,
  source: std::sync::Arc<io::Error>,
}

impl PerThreadError {
  /// The zero-based worker index.
  pub fn worker_id(&self) -> usize {
    self.worker_id
  }

  pub fn kind(&self) -> PerThreadErrorKind {
    self.kind
  }

  pub(crate) fn new(worker_id: usize, kind: PerThreadErrorKind, error: io::Error) -> Self {
    Self {
      worker_id,
      kind,
      source: std::sync::Arc::new(error),
    }
  }

  pub(crate) fn panic(worker_id: usize, payload: &(dyn Any + Send)) -> Self {
    let message = payload
      .downcast_ref::<String>()
      .map(String::as_str)
      .or_else(|| payload.downcast_ref::<&str>().copied())
      .unwrap_or("non-string panic payload");

    Self::new(
      worker_id,
      PerThreadErrorKind::Panic,
      io::Error::other(message.to_owned()),
    )
  }
}

impl fmt::Display for PerThreadError {
  fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(
      formatter,
      "per-thread worker {} ({:?}): {}",
      self.worker_id, self.kind, self.source
    )
  }
}

impl std::error::Error for PerThreadError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    Some(self.source.as_ref())
  }
}

impl From<PerThreadError> for io::Error {
  fn from(error: PerThreadError) -> Self {
    Self::new(error.source.kind(), error)
  }
}
