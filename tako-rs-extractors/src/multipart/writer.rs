use std::fs::File;
use std::io;
use std::io::Write;
use std::path::PathBuf;

use bytes::Bytes;
use uuid::Uuid;

use super::cleanup::TempFileCleanup;
use super::runtime::blocking;

/// File and guard travel together through blocking operations, including cancelled ones.
pub(super) struct UploadWriter {
  file: File,
  path: PathBuf,
  size: u64,
  cleanup: TempFileCleanup,
}

impl UploadWriter {
  pub(super) async fn create() -> io::Result<Self> {
    blocking(Self::create_blocking).await
  }

  fn create_blocking() -> io::Result<Self> {
    // Client names must never influence a filesystem path.
    let path = std::env::temp_dir().join(format!("upload-{}", Uuid::new_v4()));
    let file = File::options().write(true).create_new(true).open(&path)?;
    Ok(Self {
      file,
      cleanup: TempFileCleanup::for_path(path.clone()),
      path,
      size: 0,
    })
  }

  pub(super) async fn write(mut self, chunk: Bytes) -> io::Result<Self> {
    blocking(move || {
      self.file.write_all(&chunk)?;
      self.size += chunk.len() as u64;
      Ok(self)
    })
    .await
  }

  pub(super) async fn finish(mut self) -> io::Result<(PathBuf, u64, TempFileCleanup)> {
    blocking(move || {
      self.file.flush()?;
      drop(self.file);
      Ok((self.path, self.size, self.cleanup))
    })
    .await
  }
}

#[cfg(test)]
mod tests;
