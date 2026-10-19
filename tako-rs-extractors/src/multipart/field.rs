use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::multipart::MultipartConfig;

/// A type built from one multipart file field.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::multipart::FromMultipartField;
/// use serde::{Serialize, Deserialize};
/// use multer::Field;
///
/// #[derive(Serialize, Deserialize)]
/// struct CustomFile {
///     name: String,
///     size: usize,
/// }
///
/// impl FromMultipartField for CustomFile {
///     async fn from_field(field: Field<'_>) -> anyhow::Result<Self> {
///         let name = field.file_name().unwrap_or("unknown").to_string();
///         let data = field.bytes().await?;
///         Ok(CustomFile {
///             name,
///             size: data.len(),
///         })
///     }
/// }
/// ```
pub trait FromMultipartField: Serialize + Sized {
  /// An error rejects the request with 400.
  fn from_field(
    field: multer::Field<'_>,
  ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send;

  /// What the typed extractor calls, with the request's resolved config; defaults to `from_field`.
  fn from_field_with(
    field: multer::Field<'_>,
    _config: &MultipartConfig,
  ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send {
    Self::from_field(field)
  }
}

/// Best-effort deletes an upload's temp file on drop unless disarmed.
///
/// `Default` is disarmed, so a value built by `Deserialize` never deletes a client-chosen path.
#[derive(Default)]
pub struct TempFileCleanup {
  path: Option<PathBuf>,
}

impl TempFileCleanup {
  fn for_path(path: PathBuf) -> Self {
    Self { path: Some(path) }
  }

  /// Keeps the file on drop, for when the caller moved or persisted it.
  pub fn disarm(&mut self) {
    self.path = None;
  }
}

impl Drop for TempFileCleanup {
  fn drop(&mut self) {
    if let Some(p) = self.path.take() {
      let _ = std::fs::remove_file(&p);
    }
  }
}

impl std::fmt::Debug for TempFileCleanup {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("TempFileCleanup")
      .field("armed", &self.path.is_some())
      .finish()
  }
}

fn fresh_upload_temp_path() -> PathBuf {
  // The client's file name must never reach the path: `..` segments would allow traversal.
  std::env::temp_dir().join(format!("upload-{}", Uuid::new_v4()))
}

/// A file field streamed to a temp file, deleted on drop unless [`UploadedFile::persist`] or
/// [`UploadedFile::disarm_cleanup`] runs first.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadedFile {
  /// The client-supplied name; it never influences `path`.
  pub file_name: Option<String>,
  /// The content type the client declared for the part.
  pub content_type: Option<String>,
  /// The temp file.
  pub path: PathBuf,
  /// Bytes written to `path`.
  pub size: u64,
  #[serde(skip, default)]
  cleanup: TempFileCleanup,
}

impl UploadedFile {
  /// Keeps the temp file on drop, for when the caller already moved it.
  pub fn disarm_cleanup(&mut self) {
    self.cleanup.disarm();
  }

  /// Renames the temp file to `dest`; on success `path` becomes `dest` and the cleanup is disarmed.
  pub async fn persist(&mut self, dest: PathBuf) -> std::io::Result<()> {
    tokio::fs::rename(&self.path, &dest).await?;
    self.path = dest;
    self.cleanup.disarm();
    Ok(())
  }
}

impl FromMultipartField for UploadedFile {
  async fn from_field(mut field: multer::Field<'_>) -> anyhow::Result<Self> {
    let original = field.file_name().map(std::borrow::ToOwned::to_owned);
    let content_type = field.content_type().map(std::string::ToString::to_string);
    let tmp_path = fresh_upload_temp_path();
    // Armed before the file exists so an error mid-write still removes it.
    let cleanup = TempFileCleanup::for_path(tmp_path.clone());
    let mut outfile = File::create(&tmp_path).await?;
    let mut bytes_written: u64 = 0;
    while let Some(chunk) = field.chunk().await? {
      outfile.write_all(&chunk).await?;
      bytes_written += chunk.len() as u64;
    }
    outfile.flush().await?;
    Ok(UploadedFile {
      file_name: original,
      content_type,
      path: tmp_path,
      size: bytes_written,
      cleanup,
    })
  }
}

/// A file field held in memory.
#[derive(Debug, Serialize, Deserialize)]
pub struct InMemoryFile {
  /// The client-supplied name.
  pub file_name: Option<String>,
  /// The content type the client declared for the part.
  pub content_type: Option<String>,
  /// The field's bytes.
  #[serde(with = "serde_bytes")]
  pub data: Vec<u8>,
}

impl FromMultipartField for InMemoryFile {
  async fn from_field(field: multer::Field<'_>) -> anyhow::Result<Self> {
    let file_name = field.file_name().map(std::borrow::ToOwned::to_owned);
    let content_type = field.content_type().map(std::string::ToString::to_string);
    let data = field.bytes().await?.to_vec();

    Ok(InMemoryFile {
      file_name,
      content_type,
      data,
    })
  }
}

/// A file field kept in memory, or spilled to disk past the config's `disk_spill_threshold`;
/// called directly through `from_field`, it reads the process-global [`MultipartConfig`].
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BufferedUploadedFile {
  Memory(InMemoryFile),
  Disk(UploadedFile),
}

impl FromMultipartField for BufferedUploadedFile {
  fn from_field(
    field: multer::Field<'_>,
  ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send {
    let threshold =
      tako_rs_core::state::get_state::<MultipartConfig>().and_then(|cfg| cfg.disk_spill_threshold);
    buffer_or_spill(field, threshold)
  }

  fn from_field_with(
    field: multer::Field<'_>,
    config: &MultipartConfig,
  ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send {
    buffer_or_spill(field, config.disk_spill_threshold)
  }
}

async fn buffer_or_spill(
  mut field: multer::Field<'_>,
  threshold: Option<u64>,
) -> anyhow::Result<BufferedUploadedFile> {
  let file_name = field.file_name().map(std::borrow::ToOwned::to_owned);
  let content_type = field.content_type().map(std::string::ToString::to_string);

  let mut buffer: Vec<u8> = Vec::new();
  let mut spilled: Option<(PathBuf, File, TempFileCleanup)> = None;
  let mut bytes_written: u64 = 0;

  while let Some(chunk) = field.chunk().await? {
    if let Some((_, ref mut f, _)) = spilled {
      f.write_all(&chunk).await?;
    } else {
      // An allocation failure becomes an error instead of aborting the process.
      buffer
        .try_reserve(chunk.len())
        .map_err(|e| anyhow::anyhow!("multipart buffer alloc failed: {e}"))?;
      buffer.extend_from_slice(&chunk);
      if let Some(t) = threshold
        && (buffer.len() as u64) > t
      {
        let path = fresh_upload_temp_path();
        let cleanup = TempFileCleanup::for_path(path.clone());
        let mut f = File::create(&path).await?;
        f.write_all(&buffer).await?;
        spilled = Some((path, f, cleanup));
        buffer.clear();
      }
    }
    bytes_written += chunk.len() as u64;
  }

  if let Some((path, mut f, cleanup)) = spilled {
    f.flush().await?;
    Ok(BufferedUploadedFile::Disk(UploadedFile {
      file_name,
      content_type,
      path,
      size: bytes_written,
      cleanup,
    }))
  } else {
    Ok(BufferedUploadedFile::Memory(InMemoryFile {
      file_name,
      content_type,
      data: buffer,
    }))
  }
}
