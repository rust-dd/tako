#![cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
//! Multipart form data extraction and file upload handling.
//!
//! This module provides extractors for parsing `multipart/form-data` request bodies,
//! commonly used for file uploads and complex form submissions. It supports both
//! raw multipart access through [`TakoMultipart`](crate::multipart::TakoMultipart) and strongly-typed extraction
//! through [`TakoTypedMultipart`](crate::multipart::TakoTypedMultipart), with built-in support for file uploads to disk
//! or memory.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::multipart::{TakoTypedMultipart, UploadedFile};
//! use serde::Deserialize;
//!
//! #[derive(Deserialize)]
//! struct FileUploadForm {
//!     title: String,
//!     description: String,
//!     file: UploadedFile,
//! }
//!
//! async fn upload_handler(
//!     TakoTypedMultipart { data: form, .. }: TakoTypedMultipart<'_, FileUploadForm, UploadedFile>
//! ) {
//!     println!("Uploaded file: {:?}", form.file.file_name);
//!     println!("File size: {} bytes", form.file.size);
//!     println!("Saved to: {:?}", form.file.path);
//! }
//! ```

use std::path::PathBuf;
use std::sync::Arc;

use http::StatusCode;
use http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use multer::Constraints;
use multer::Multipart;
use multer::SizeLimit;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Map;
use serde_json::Value;
use tako_core::extractors::FromRequest;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

/// Per-route or global configuration for multipart extraction.
///
/// Insert into request extensions (or set as global state) to constrain how
/// `TakoMultipart` / `TakoTypedMultipart` consume request bodies. Defaults
/// are permissive — opt in to limits explicitly.
#[derive(Debug, Clone)]
pub struct MultipartConfig {
  /// Total request body cap, in bytes. `None` = no whole-payload limit.
  pub total_size_limit: Option<u64>,
  /// Per-part body cap, in bytes. `None` = no per-part limit.
  ///
  /// Defaults to 1 MiB to keep text-field deserialization (`from_utf8` on a
  /// fully-collected `Vec<u8>`) bounded; raise it explicitly when handling
  /// genuinely large fields.
  pub per_part_size_limit: Option<u64>,
  /// Maximum number of parts. Reaching this returns an error mid-parse.
  ///
  /// Enforced by [`TakoTypedMultipart`]; the raw [`TakoMultipart`] does not
  /// enforce this because users may consume the inner `multer::Multipart`
  /// directly. Prefer the typed extractor when you need the cap.
  pub max_parts: Option<usize>,
  /// Allow-list of part content-types (e.g. `image/png`, `application/pdf`).
  /// `None` (or empty) = accept any.
  pub allowed_content_types: Option<Arc<Vec<String>>>,
  /// When uploading via `UploadedFile`, switch from in-memory buffering to a
  /// temp file once the part exceeds this many bytes. `None` = always disk.
  pub disk_spill_threshold: Option<u64>,
  /// Maximum time to wait for a single chunk from a multipart field before
  /// aborting the request. Protects against slow-read style `DoS` where the
  /// client drips a few bytes per second to hold a parser permit open.
  /// `None` = no per-chunk timeout. Applied by [`TakoTypedMultipart`].
  pub field_chunk_timeout: Option<std::time::Duration>,
}

impl Default for MultipartConfig {
  fn default() -> Self {
    Self {
      total_size_limit: None,
      // Bound the per-part allocation by default so an unconfigured
      // application doesn't OOM on a hostile multipart upload.
      per_part_size_limit: Some(1024 * 1024),
      max_parts: None,
      allowed_content_types: None,
      disk_spill_threshold: None,
      field_chunk_timeout: None,
    }
  }
}

impl MultipartConfig {
  /// Build a permissive config (no limits). Configure via the builder methods.
  pub fn new() -> Self {
    Self::default()
  }

  /// Set a whole-request body cap.
  pub fn total_size_limit(mut self, bytes: u64) -> Self {
    self.total_size_limit = Some(bytes);
    self
  }

  /// Set a per-part body cap.
  pub fn per_part_size_limit(mut self, bytes: u64) -> Self {
    self.per_part_size_limit = Some(bytes);
    self
  }

  /// Set the maximum number of parts.
  pub fn max_parts(mut self, n: usize) -> Self {
    self.max_parts = Some(n);
    self
  }

  /// Maximum time to wait for a single chunk from any multipart field. See
  /// [`Self::field_chunk_timeout`].
  pub fn field_chunk_timeout(mut self, d: std::time::Duration) -> Self {
    self.field_chunk_timeout = Some(d);
    self
  }

  /// Replace the allow-list of accepted part content-types.
  pub fn allowed_content_types<I, S>(mut self, types: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.allowed_content_types = Some(Arc::new(types.into_iter().map(Into::into).collect()));
    self
  }

  /// Set the in-memory → disk spill threshold for `UploadedFile`.
  pub fn disk_spill_threshold(mut self, bytes: u64) -> Self {
    self.disk_spill_threshold = Some(bytes);
    self
  }

  fn to_constraints(&self) -> Constraints {
    let mut limit = SizeLimit::new();
    if let Some(b) = self.total_size_limit {
      limit = limit.whole_stream(b);
    }
    if let Some(b) = self.per_part_size_limit {
      limit = limit.per_field(b);
    }
    Constraints::new().size_limit(limit)
  }

  fn lookup(req_ext: &http::Extensions) -> MultipartConfig {
    if let Some(cfg) = req_ext.get::<MultipartConfig>() {
      return cfg.clone();
    }
    if let Some(arc) = tako_core::state::get_state::<MultipartConfig>() {
      return arc.as_ref().clone();
    }
    MultipartConfig::default()
  }

  fn content_type_ok(&self, ct: Option<&str>) -> bool {
    let Some(allow) = self.allowed_content_types.as_ref() else {
      return true;
    };
    if allow.is_empty() {
      return true;
    }
    let ct = ct.unwrap_or("");
    allow.iter().any(|a| ct.starts_with(a.as_str()))
  }
}

/// Error type for multipart extraction.
#[derive(Debug)]
#[non_exhaustive]
pub enum MultipartError {
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Content-Type header is not multipart/form-data.
  InvalidContentType,
  /// Content-Type header contains invalid UTF-8 sequences.
  InvalidUtf8,
  /// Failed to parse boundary from Content-Type header.
  BoundaryParseError(String),
  /// A part's content-type is not in the configured allow-list.
  DisallowedContentType(String),
  /// The configured `max_parts` count was exceeded.
  TooManyParts,
}

impl Responder for MultipartError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> tako_core::types::Response {
    match self {
      MultipartError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing Content-Type header").into_response()
      }
      MultipartError::InvalidContentType => {
        (StatusCode::BAD_REQUEST, "Invalid Content-Type header").into_response()
      }
      MultipartError::InvalidUtf8 => (
        StatusCode::BAD_REQUEST,
        "Content-Type header contains invalid UTF-8",
      )
        .into_response(),
      MultipartError::BoundaryParseError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Not multipart/form-data or boundary missing: {err}"),
      )
        .into_response(),
      MultipartError::DisallowedContentType(ct) => (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("part content-type not allowed: {ct}"),
      )
        .into_response(),
      MultipartError::TooManyParts => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "too many multipart parts in request",
      )
        .into_response(),
    }
  }
}

/// Error type for typed multipart extraction.
#[derive(Debug)]
#[non_exhaustive]
pub enum TypedMultipartError {
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Content-Type header is not multipart/form-data.
  InvalidContentType,
  /// Content-Type header contains invalid UTF-8 sequences.
  InvalidUtf8,
  /// Failed to parse boundary from Content-Type header.
  BoundaryParseError(String),
  /// Error processing a multipart field.
  FieldError(String),
  /// Failed to deserialize form data into the target type.
  DeserializationError(String),
  /// I/O error occurred during processing.
  IoError(String),
  /// A part's content-type is not in the configured allow-list.
  DisallowedContentType(String),
  /// The configured `max_parts` count was exceeded.
  TooManyParts,
}

impl Responder for TypedMultipartError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> tako_core::types::Response {
    match self {
      TypedMultipartError::MissingContentType => {
        (StatusCode::BAD_REQUEST, "Missing Content-Type header").into_response()
      }
      TypedMultipartError::InvalidContentType => {
        (StatusCode::BAD_REQUEST, "Invalid Content-Type header").into_response()
      }
      TypedMultipartError::InvalidUtf8 => (
        StatusCode::BAD_REQUEST,
        "Content-Type header contains invalid UTF-8",
      )
        .into_response(),
      TypedMultipartError::BoundaryParseError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Not multipart/form-data or boundary missing: {err}"),
      )
        .into_response(),
      TypedMultipartError::FieldError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Field processing error: {err}"),
      )
        .into_response(),
      TypedMultipartError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Deserialization error: {err}"),
      )
        .into_response(),
      TypedMultipartError::IoError(err) => (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("IO error: {err}"),
      )
        .into_response(),
      TypedMultipartError::DisallowedContentType(ct) => (
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        format!("part content-type not allowed: {ct}"),
      )
        .into_response(),
      TypedMultipartError::TooManyParts => (
        StatusCode::PAYLOAD_TOO_LARGE,
        "too many multipart parts in request",
      )
        .into_response(),
    }
  }
}

/// Wrapper around `multer::Multipart` to provide additional functionality.
///
/// This wrapper provides a unified interface for processing multipart form data
/// while maintaining compatibility with the underlying `multer` crate. It can be
/// used for manual processing of multipart fields when more control is needed
/// than the typed multipart extractor provides.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::extractors::multipart::TakoMultipart;
/// use tako::extractors::FromRequest;
/// use tako::types::Request;
///
/// async fn manual_multipart_handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
///     let TakoMultipart(mut multipart) = TakoMultipart::from_request(&mut req).await?;
///
///     while let Some(field) = multipart.next_field().await? {
///         if let Some(name) = field.name() {
///             println!("Field name: {}", name);
///             if let Some(filename) = field.file_name() {
///                 println!("File: {}", filename);
///             }
///         }
///     }
///
///     Ok(())
/// }
/// ```
#[doc(alias = "multipart")]
pub struct TakoMultipart<'a>(pub Multipart<'a>);

impl<'a> TakoMultipart<'a> {
  /// Consumes the wrapper and returns the inner `Multipart` instance.
  #[inline]
  pub fn into_inner(self) -> Multipart<'a> {
    self.0
  }
}

impl<'a> FromRequest<'a> for TakoMultipart<'a> {
  type Error = MultipartError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_multipart(req))
  }
}

impl<'a> TakoMultipart<'a> {
  fn extract_multipart(req: &'a mut Request) -> Result<TakoMultipart<'a>, MultipartError> {
    let content_type = req
      .headers()
      .get(CONTENT_TYPE)
      .ok_or(MultipartError::MissingContentType)?;

    let content_type_str = content_type
      .to_str()
      .map_err(|_| MultipartError::InvalidUtf8)?;

    let boundary = multer::parse_boundary(content_type_str)
      .map_err(|e| MultipartError::BoundaryParseError(e.to_string()))?;

    let cfg = MultipartConfig::lookup(req.extensions());
    let constraints = cfg.to_constraints();
    let body_stream = req.body_mut().into_data_stream();
    Ok(TakoMultipart(Multipart::with_constraints(
      body_stream,
      boundary,
      constraints,
    )))
  }
}

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
/// disk. The threshold comes from the active [`MultipartConfig`]; without one
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
    let cfg = tako_core::state::get_state::<MultipartConfig>().map(|a| a.as_ref().clone());
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

/// Represents a strongly-typed multipart request.
///
/// This struct allows deserialization of multipart form data into a strongly-typed
/// structure, combining both file and text fields. It provides automatic handling
/// of different field types and deserializes the entire form into a single data structure.
///
/// # Type Parameters
///
/// * `T` - The target type to deserialize form data into
/// * `F` - The type used for file fields (must implement `FromMultipartField`)
#[doc(alias = "typed_multipart")]
pub struct TakoTypedMultipart<'a, T, F> {
  /// Deserialized data from the multipart request.
  pub data: T,
  /// Marker for the field type (used for type inference).
  _marker: core::marker::PhantomData<&'a F>,
}

impl<'a, T, F> FromRequest<'a> for TakoTypedMultipart<'a, T, F>
where
  T: DeserializeOwned + 'static,
  F: FromMultipartField + serde::Serialize + 'static,
{
  type Error = TypedMultipartError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    async move {
      let content_type = req
        .headers()
        .get(CONTENT_TYPE)
        .ok_or(TypedMultipartError::MissingContentType)?;

      let content_type_str = content_type
        .to_str()
        .map_err(|_| TypedMultipartError::InvalidUtf8)?;

      let boundary = multer::parse_boundary(content_type_str)
        .map_err(|e| TypedMultipartError::BoundaryParseError(e.to_string()))?;

      let cfg = MultipartConfig::lookup(req.extensions());
      let constraints = cfg.to_constraints();
      let mut multipart =
        Multipart::with_constraints(req.body_mut().into_data_stream(), boundary, constraints);
      let mut map = Map::<String, Value>::new();
      let mut count: usize = 0;

      let field_timeout = cfg.field_chunk_timeout;
      loop {
        let next_field_fut = multipart.next_field();
        let field = match field_timeout {
          Some(d) => match tokio::time::timeout(d, next_field_fut).await {
            Ok(Ok(field)) => field,
            Ok(Err(e)) => return Err(TypedMultipartError::FieldError(e.to_string())),
            Err(_) => {
              return Err(TypedMultipartError::FieldError(
                "multipart slow-read timeout".to_string(),
              ));
            }
          },
          None => next_field_fut
            .await
            .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?,
        };
        let Some(field) = field else {
          break;
        };
        count += 1;
        if let Some(max) = cfg.max_parts
          && count > max
        {
          return Err(TypedMultipartError::TooManyParts);
        }
        let part_ct = field.content_type().map(std::string::ToString::to_string);
        if !cfg.content_type_ok(part_ct.as_deref()) {
          return Err(TypedMultipartError::DisallowedContentType(
            part_ct.unwrap_or_default(),
          ));
        }

        let field_name = field
          .name()
          .ok_or_else(|| TypedMultipartError::FieldError("Field name missing".to_string()))?
          .to_owned();

        if field.file_name().is_some() {
          let file_value: F = match field_timeout {
            Some(d) => match tokio::time::timeout(d, F::from_field(field)).await {
              Ok(Ok(v)) => v,
              Ok(Err(e)) => return Err(TypedMultipartError::FieldError(e.to_string())),
              Err(_) => {
                return Err(TypedMultipartError::FieldError(
                  "multipart slow-read timeout".to_string(),
                ));
              }
            },
            None => F::from_field(field)
              .await
              .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?,
          };

          let json_value = serde_json::to_value(file_value)
            .map_err(|e| TypedMultipartError::DeserializationError(e.to_string()))?;

          map.insert(field_name, json_value);
        } else {
          let field_bytes = match field_timeout {
            Some(d) => match tokio::time::timeout(d, field.bytes()).await {
              Ok(Ok(b)) => b,
              Ok(Err(e)) => return Err(TypedMultipartError::FieldError(e.to_string())),
              Err(_) => {
                return Err(TypedMultipartError::FieldError(
                  "multipart slow-read timeout".to_string(),
                ));
              }
            },
            None => field
              .bytes()
              .await
              .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?,
          };

          let text = String::from_utf8(field_bytes.to_vec())
            .map_err(|_| TypedMultipartError::InvalidUtf8)?;

          map.insert(field_name, Value::String(text));
        }
      }

      let data: T = serde_json::from_value(Value::Object(map))
        .map_err(|e| TypedMultipartError::DeserializationError(e.to_string()))?;

      Ok(Self {
        data,
        _marker: core::marker::PhantomData,
      })
    }
  }
}
