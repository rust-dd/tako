#![cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
//! Multipart form data extraction and file upload handling.
//!
//! This module provides extractors for parsing `multipart/form-data` request bodies,
//! commonly used for file uploads and complex form submissions. It supports both
//! raw multipart access through [`TakoMultipart`](crate::extractors::multipart::TakoMultipart) and strongly-typed extraction
//! through [`TakoTypedMultipart`](crate::extractors::multipart::TakoTypedMultipart), with built-in support for file uploads to disk
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

use http::StatusCode;
use http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use multer::Multipart;
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Map;
use serde_json::Value;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

use crate::extractors::FromRequest;
use crate::responder::Responder;
use crate::types::Request;

/// Error type for multipart extraction.
#[derive(Debug)]
pub enum MultipartError {
  /// Content-Type header is missing from the request.
  MissingContentType,
  /// Content-Type header is not multipart/form-data.
  InvalidContentType,
  /// Content-Type header contains invalid UTF-8 sequences.
  InvalidUtf8,
  /// Failed to parse boundary from Content-Type header.
  BoundaryParseError(String),
}

impl Responder for MultipartError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> crate::types::Response {
    let message = match self {
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
        format!("Not multipart/form-data or boundary missing: {}", err),
      )
        .into_response(),
    };
    message
  }
}

/// Error type for typed multipart extraction.
#[derive(Debug)]
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
}

impl Responder for TypedMultipartError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> crate::types::Response {
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
        format!("Not multipart/form-data or boundary missing: {}", err),
      )
        .into_response(),
      TypedMultipartError::FieldError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Field processing error: {}", err),
      )
        .into_response(),
      TypedMultipartError::DeserializationError(err) => (
        StatusCode::BAD_REQUEST,
        format!("Deserialization error: {}", err),
      )
        .into_response(),
      TypedMultipartError::IoError(err) => (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("IO error: {}", err),
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

    let body_stream = req.body_mut().into_data_stream();
    Ok(TakoMultipart(Multipart::new(body_stream, boundary)))
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

/// Represents a file uploaded to the server and saved to disk.
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
}

impl FromMultipartField for UploadedFile {
  /// Creates an `UploadedFile` instance from a multipart field.
  async fn from_field(mut field: multer::Field<'_>) -> anyhow::Result<Self> {
    let original = field.file_name().map(|s| s.to_owned());
    let content_type = field.content_type().map(|m| m.to_string());
    let tmp_path = {
      let fname = original
        .as_deref()
        .map(|f| format!("upload-{}-{}", Uuid::new_v4(), f))
        .unwrap_or_else(|| format!("upload-{}", Uuid::new_v4()));
      std::env::temp_dir().join(fname)
    };
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
    let file_name = field.file_name().map(|s| s.to_owned());
    let content_type = field.content_type().map(|m| m.to_string());
    let data = field.bytes().await?.to_vec();

    Ok(InMemoryFile {
      file_name,
      content_type,
      data,
    })
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

      let mut multipart = Multipart::new(req.body_mut().into_data_stream(), boundary);
      let mut map = Map::<String, Value>::new();

      while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?
      {
        let field_name = field
          .name()
          .ok_or_else(|| TypedMultipartError::FieldError("Field name missing".to_string()))?
          .to_owned();

        if field.file_name().is_some() {
          let file_value: F = F::from_field(field)
            .await
            .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?;

          let json_value = serde_json::to_value(file_value)
            .map_err(|e| TypedMultipartError::DeserializationError(e.to_string()))?;

          map.insert(field_name, json_value);
        } else {
          let field_bytes = field
            .bytes()
            .await
            .map_err(|e| TypedMultipartError::FieldError(e.to_string()))?;

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
