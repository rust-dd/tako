use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::multipart::MultipartConfig;

/// Trait for types that can be constructed from a multipart field.
///
/// This trait allows custom types to define how they should be created from
/// individual multipart fields, enabling flexible handling of different field
/// types including files and text data.
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
  /// Constructs an instance of the type from a `multer::Field`.
  fn from_field(
    field: multer::Field<'_>,
  ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send;
}

/// RAII cleanup for a temp file produced by the multipart upload pipeline.
///
/// When this guard drops with a `Some(path)`, the file at that path is best-effort
/// unlinked. Default is `None` (no cleanup) so that values obtained via
/// `Deserialize` do not delete arbitrary paths supplied by an attacker.
#[derive(Default)]
pub struct TempFileCleanup {
  path: Option<PathBuf>,
}

impl TempFileCleanup {
  fn for_path(path: PathBuf) -> Self {
    Self { path: Some(path) }
  }

  /// Disarm the cleanup — the temp file will not be removed on Drop.
  /// Use this when the caller has moved or persisted the file elsewhere.
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
  // UUID-only basename: the client-supplied filename is preserved in
  // `UploadedFile.file_name` but MUST NOT influence the on-disk path
  // (`..` segments would otherwise enable path traversal).
  std::env::temp_dir().join(format!("upload-{}", Uuid::new_v4()))
}

/// Represents a file uploaded to the server and saved to disk.
///
/// The on-disk temp file is auto-removed when the `UploadedFile` is dropped
/// (RAII). Call [`UploadedFile::persist`] or [`UploadedFile::disarm_cleanup`]
/// before drop if you want to keep the file.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadedFile {
  /// Original file name provided by the client, if any.
  pub file_name: Option<String>,
  /// MIME type of the uploaded file, if provided.
  pub content_type: Option<String>,
  /// Path to the saved file on disk.
  pub path: PathBuf,
  /// Size of the uploaded file in bytes.
  pub size: u64,
  #[serde(skip, default)]
  cleanup: TempFileCleanup,
}

impl UploadedFile {
  /// Disarm the RAII cleanup. The temp file will remain on disk after drop.
  /// Use this when you have already moved the file elsewhere.
  pub fn disarm_cleanup(&mut self) {
    self.cleanup.disarm();
  }

  /// Persist the temp file by renaming it to `dest`. On success the cleanup
  /// is disarmed and `self.path` is updated to `dest`.
  pub async fn persist(&mut self, dest: PathBuf) -> std::io::Result<()> {
    tokio::fs::rename(&self.path, &dest).await?;
    self.path = dest;
    self.cleanup.disarm();
    Ok(())
  }
}

impl FromMultipartField for UploadedFile {
  /// Creates an `UploadedFile` instance from a multipart field.
  async fn from_field(mut field: multer::Field<'_>) -> anyhow::Result<Self> {
    let original = field.file_name().map(std::borrow::ToOwned::to_owned);
    let content_type = field.content_type().map(std::string::ToString::to_string);
    let tmp_path = fresh_upload_temp_path();
    // Register cleanup BEFORE opening the file so an error mid-write still
    // removes the partial file on early return.
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

/// Represents a file uploaded to the server and stored in memory.
#[derive(Debug, Serialize, Deserialize)]
pub struct InMemoryFile {
  /// Original file name provided by the client, if any.
  pub file_name: Option<String>,
  /// MIME type of the uploaded file, if provided.
  pub content_type: Option<String>,
  /// File content stored as a byte array.
  #[serde(with = "serde_bytes")]
  pub data: Vec<u8>,
}

impl FromMultipartField for InMemoryFile {
  /// Creates an `InMemoryFile` instance from a multipart field.
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

/// File upload that keeps small payloads in memory and spills large ones to
/// disk. The threshold comes from the active [`MultipartConfig`](crate::multipart::MultipartConfig); without one
/// it always keeps the file in memory.
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum BufferedUploadedFile {
  /// Payload stayed in memory.
  Memory(InMemoryFile),
  /// Payload was streamed to disk because it exceeded the threshold.
  Disk(UploadedFile),
}

impl FromMultipartField for BufferedUploadedFile {
  async fn from_field(mut field: multer::Field<'_>) -> anyhow::Result<Self> {
    let cfg = tako_rs_core::state::get_state::<MultipartConfig>().map(|a| a.as_ref().clone());
    let threshold = cfg.as_ref().and_then(|c| c.disk_spill_threshold);

    let file_name = field.file_name().map(std::borrow::ToOwned::to_owned);
    let content_type = field.content_type().map(std::string::ToString::to_string);

    let mut buffer: Vec<u8> = Vec::new();
    let mut spilled: Option<(PathBuf, File, TempFileCleanup)> = None;
    let mut bytes_written: u64 = 0;

    while let Some(chunk) = field.chunk().await? {
      if let Some((_, ref mut f, _)) = spilled {
        f.write_all(&chunk).await?;
      } else {
        // Try-reserve: a hostile client could send a series of small chunks
        // whose cumulative size eventually exceeds available memory.
        // `extend_from_slice` would abort the process on alloc failure;
        // `try_reserve` lets us bail out with a normal error and a 4xx-style
        // response from the caller.
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
}
