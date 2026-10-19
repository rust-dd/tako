use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

pub use super::cleanup::TempFileCleanup;
use super::runtime;
use super::writer::UploadWriter;
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

  /// Transfers a temp-file guard through typed deserialization. Custom wrappers around
  /// [`UploadedFile`] should delegate to the wrapped file; other types can use the default.
  fn take_cleanup(&mut self) -> TempFileCleanup {
    TempFileCleanup::default()
  }
}

/// A file field streamed to a temp file, deleted on drop unless [`UploadedFile::persist`] or
/// [`UploadedFile::disarm_cleanup`] runs first.
#[derive(Debug, Serialize)]
pub struct UploadedFile {
  /// The client-supplied name; it never influences `path`.
  pub file_name: Option<String>,
  /// The content type the client declared for the part.
  pub content_type: Option<String>,
  /// The temp file.
  pub path: PathBuf,
  /// Bytes written to `path`.
  pub size: u64,
  #[serde(skip)]
  cleanup: TempFileCleanup,
}

impl<'de> Deserialize<'de> for UploadedFile {
  fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
    #[derive(Deserialize)]
    #[serde(rename = "UploadedFile")]
    struct Metadata {
      file_name: Option<String>,
      content_type: Option<String>,
      path: PathBuf,
      size: u64,
    }

    let metadata = Metadata::deserialize(deserializer)?;
    Ok(Self {
      cleanup: TempFileCleanup::claim(&metadata.path),
      file_name: metadata.file_name,
      content_type: metadata.content_type,
      path: metadata.path,
      size: metadata.size,
    })
  }
}

impl UploadedFile {
  /// Keeps the temp file on drop, for when the caller already moved it.
  pub fn disarm_cleanup(&mut self) {
    self.cleanup.disarm();
  }

  /// Renames the temp file to `dest`; on success `path` becomes `dest` and the cleanup is disarmed.
  pub async fn persist(&mut self, dest: PathBuf) -> std::io::Result<()> {
    let source = self.path.clone();
    self.path = runtime::blocking(move || {
      std::fs::rename(source, &dest)?;
      Ok(dest)
    })
    .await?;
    self.cleanup.disarm();
    Ok(())
  }
}

impl FromMultipartField for UploadedFile {
  fn take_cleanup(&mut self) -> TempFileCleanup {
    std::mem::take(&mut self.cleanup)
  }

  async fn from_field(mut field: multer::Field<'_>) -> anyhow::Result<Self> {
    let original = field.file_name().map(std::borrow::ToOwned::to_owned);
    let content_type = field.content_type().map(std::string::ToString::to_string);
    let mut outfile = UploadWriter::create().await?;
    while let Some(chunk) = field.chunk().await? {
      outfile = outfile.write(chunk).await?;
    }
    let (path, size, cleanup) = outfile.finish().await?;
    Ok(UploadedFile {
      file_name: original,
      content_type,
      path,
      size,
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
  fn take_cleanup(&mut self) -> TempFileCleanup {
    match self {
      Self::Memory(_) => TempFileCleanup::default(),
      Self::Disk(file) => file.take_cleanup(),
    }
  }

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

  let mut buffer = Vec::new();
  let mut spilled = None::<UploadWriter>;

  while let Some(chunk) = field.chunk().await? {
    if let Some(file) = spilled.take() {
      spilled = Some(file.write(chunk).await?);
    } else {
      // An allocation failure becomes an error instead of aborting the process.
      buffer
        .try_reserve(chunk.len())
        .map_err(|e| anyhow::anyhow!("multipart buffer alloc failed: {e}"))?;
      buffer.extend_from_slice(&chunk);
      if let Some(t) = threshold
        && (buffer.len() as u64) > t
      {
        let file = UploadWriter::create().await?;
        spilled = Some(file.write(std::mem::take(&mut buffer).into()).await?);
      }
    }
  }

  if let Some(file) = spilled {
    let (path, size, cleanup) = file.finish().await?;
    Ok(BufferedUploadedFile::Disk(UploadedFile {
      file_name,
      content_type,
      path,
      size,
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
