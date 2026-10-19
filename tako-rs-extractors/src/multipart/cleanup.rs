use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

/// Best-effort deletes an upload's temp file on drop unless disarmed.
///
/// `Default` is disarmed. Deserialization only acquires cleanup for files owned by the
/// current typed multipart extraction; client-supplied paths do not acquire cleanup.
#[derive(Default)]
pub struct TempFileCleanup {
  file: Option<Arc<TempFile>>,
}

impl TempFileCleanup {
  pub(super) fn for_path(path: PathBuf) -> Self {
    Self {
      file: Some(Arc::new(TempFile {
        path,
        armed: AtomicBool::new(true),
      })),
    }
  }

  /// Keeps the file on drop, for when the caller moved or persisted it.
  pub fn disarm(&mut self) {
    if let Some(file) = self.file.take() {
      file.armed.store(false, Ordering::Relaxed);
    }
  }

  pub(super) fn claim(path: &Path) -> Self {
    CURRENT.with(|current| Self {
      file: current
        .borrow()
        .files
        .get(path)
        .and_then(|cleanup| cleanup.file.clone()),
    })
  }
}

struct TempFile {
  path: PathBuf,
  armed: AtomicBool,
}

impl Drop for TempFile {
  fn drop(&mut self) {
    if *self.armed.get_mut() {
      let _ = std::fs::remove_file(&self.path);
    }
  }
}

impl std::fmt::Debug for TempFileCleanup {
  fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    formatter
      .debug_struct("TempFileCleanup")
      .field(
        "armed",
        &self
          .file
          .as_ref()
          .is_some_and(|file| file.armed.load(Ordering::Relaxed)),
      )
      .finish()
  }
}

/// Keeps uploads owned while their metadata passes through Serde. Only the synchronous
/// final deserialization can claim guards, so concurrent requests cannot share this scope.
#[derive(Default)]
pub(super) struct PendingUploads {
  files: HashMap<PathBuf, TempFileCleanup>,
}

impl PendingUploads {
  pub(super) fn insert(&mut self, cleanup: TempFileCleanup) {
    if let Some(file) = &cleanup.file {
      self.files.insert(file.path.clone(), cleanup);
    }
  }

  pub(super) fn deserialize<T: serde::de::DeserializeOwned>(
    self,
    value: serde_json::Value,
  ) -> serde_json::Result<T> {
    let _scope = CleanupScope {
      previous: CURRENT.with(|current| current.replace(self)),
    };
    serde_json::from_value(value)
  }
}

thread_local! {
  static CURRENT: RefCell<PendingUploads> = RefCell::new(PendingUploads::default());
}

struct CleanupScope {
  previous: PendingUploads,
}

impl Drop for CleanupScope {
  fn drop(&mut self) {
    let remaining = CURRENT.with(|current| current.replace(std::mem::take(&mut self.previous)));
    drop(remaining);
  }
}

#[cfg(test)]
mod tests;
