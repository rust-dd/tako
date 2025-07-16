//! Multipart form data extraction and file upload handling.
//!
//! This module provides extractors for parsing `multipart/form-data` request bodies,
//! commonly used for file uploads and complex form submissions. It supports both
//! raw multipart access through [`TakoMultipart`] and strongly-typed extraction
//! through [`TakoTypedMultipart`], with built-in support for file uploads to disk
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

use http::{StatusCode, header::CONTENT_TYPE};
use http_body_util::BodyExt;
use multer::Multipart;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use std::future::ready;
use tokio::{fs::File, io::AsyncWriteExt};
use uuid::Uuid;

use crate::{extractors::FromRequest, responder::Responder, types::Request};

/// Error type for multipart extraction.
///
/// Represents various failure modes that can occur when extracting multipart
/// form data from HTTP request bodies.
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
    ///
    /// Maps multipart extraction errors to appropriate HTTP status codes with
    /// descriptive error messages. All errors result in `400 Bad Request` as they
    /// indicate client-side issues with the request format.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::multipart::MultipartError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = MultipartError::MissingContentType;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    ///
    /// let error = MultipartError::InvalidContentType;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    /// ```
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
///
/// Represents various failure modes that can occur when extracting and
/// deserializing typed multipart form data from HTTP request bodies.
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
    ///
    /// Maps typed multipart extraction errors to appropriate HTTP status codes with
    /// descriptive error messages. Most errors result in `400 Bad Request`, while
    /// I/O errors result in `500 Internal Server Error`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::multipart::TypedMultipartError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = TypedMultipartError::FieldError("Invalid field".to_string());
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    ///
    /// let error = TypedMultipartError::IoError("Disk full".to_string());
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    /// ```
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
pub struct TakoMultipart<'a>(pub Multipart<'a>);

impl<'a> TakoMultipart<'a> {
    /// Consumes the wrapper and returns the inner `Multipart` instance.
    ///
    /// This allows direct access to the underlying `multer::Multipart` for
    /// advanced use cases that require functionality not exposed by the wrapper.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::multipart::TakoMultipart;
    /// use multer::Multipart;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// # let body_stream = futures::stream::empty();
    /// # let boundary = "boundary".to_string();
    /// let multipart = Multipart::new(body_stream, boundary);
    /// let wrapper = TakoMultipart(multipart);
    ///
    /// let inner: Multipart = wrapper.into_inner();
    /// // Use inner multipart directly with multer APIs
    /// # Ok(())
    /// # }
    /// ```
    #[inline]
    pub fn into_inner(self) -> Multipart<'a> {
        self.0
    }
}

impl<'a> FromRequest<'a> for TakoMultipart<'a> {
    type Error = MultipartError;

    /// Extracts a `TakoMultipart` instance from an HTTP request.
    ///
    /// This function validates the Content-Type header, parses the boundary parameter,
    /// and creates a `Multipart` instance from the request body for manual processing.
    ///
    /// # Errors
    ///
    /// Returns `MultipartError` if:
    /// - Content-Type header is missing or invalid
    /// - Boundary parameter cannot be parsed
    /// - Content-Type is not multipart/form-data
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, multipart::TakoMultipart};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let TakoMultipart(mut multipart) = TakoMultipart::from_request(&mut req).await?;
    ///
    ///     // Process multipart fields manually
    ///     while let Some(field) = multipart.next_field().await? {
    ///         println!("Processing field: {:?}", field.name());
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_multipart(req))
    }
}

impl<'a> TakoMultipart<'a> {
    /// Extracts multipart data from the request.
    ///
    /// Internal method that handles the actual extraction logic including
    /// Content-Type validation and boundary parsing.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request to extract multipart data from
    ///
    /// # Errors
    ///
    /// Returns `MultipartError` if extraction fails for any reason.
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
    ///
    /// This method is called for each multipart field that should be deserialized
    /// into this type. Implementations should handle reading the field data and
    /// creating the appropriate instance.
    ///
    /// # Arguments
    ///
    /// * `field` - The multipart field to process
    ///
    /// # Errors
    ///
    /// Should return an error if the field cannot be processed or converted
    /// into the target type.
    fn from_field(
        field: multer::Field<'_>,
    ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send;
}

/// Represents a file uploaded to the server and saved to disk.
///
/// This struct stores metadata about an uploaded file that has been saved to
/// the filesystem. It includes the original filename, content type, file size,
/// and the path where the file was saved.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::multipart::UploadedFile;
/// use std::path::PathBuf;
///
/// let uploaded = UploadedFile {
///     file_name: Some("document.pdf".to_string()),
///     content_type: Some("application/pdf".to_string()),
///     path: PathBuf::from("/tmp/upload-123.pdf"),
///     size: 1024,
/// };
///
/// assert_eq!(uploaded.file_name.as_deref(), Some("document.pdf"));
/// assert_eq!(uploaded.size, 1024);
/// ```
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
    ///
    /// The file is saved to a temporary directory with a unique filename that
    /// includes a UUID to prevent naming conflicts. The original filename is
    /// preserved in the metadata when available.
    ///
    /// # Arguments
    ///
    /// * `field` - The multipart field containing the file data
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The temporary file cannot be created
    /// - Writing to the file fails
    /// - I/O operations fail
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::multipart::{FromMultipartField, UploadedFile};
    /// use multer::Field;
    ///
    /// async fn process_file_field(field: Field<'_>) -> anyhow::Result<()> {
    ///     let uploaded = UploadedFile::from_field(field).await?;
    ///     println!("File saved to: {:?}", uploaded.path);
    ///     println!("File size: {} bytes", uploaded.size);
    ///     Ok(())
    /// }
    /// ```
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
///
/// This struct stores an uploaded file entirely in memory as a byte array,
/// along with metadata about the file. It's suitable for small files or
/// when you need immediate access to the file content without disk I/O.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::multipart::InMemoryFile;
///
/// let file = InMemoryFile {
///     file_name: Some("config.json".to_string()),
///     content_type: Some("application/json".to_string()),
///     data: b"{\"key\": \"value\"}".to_vec(),
/// };
///
/// assert_eq!(file.data.len(), 16);
/// assert_eq!(file.file_name.as_deref(), Some("config.json"));
/// ```
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
    ///
    /// The entire file content is read into memory as a byte vector. This is
    /// suitable for small files but should be used carefully with large files
    /// to avoid excessive memory usage.
    ///
    /// # Arguments
    ///
    /// * `field` - The multipart field containing the file data
    ///
    /// # Errors
    ///
    /// Returns an error if reading the field data fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::multipart::{FromMultipartField, InMemoryFile};
    /// use multer::Field;
    ///
    /// async fn process_small_file(field: Field<'_>) -> anyhow::Result<()> {
    ///     let file = InMemoryFile::from_field(field).await?;
    ///     println!("File content: {} bytes", file.data.len());
    ///
    ///     // Process file data directly from memory
    ///     if let Some(name) = &file.file_name {
    ///         if name.ends_with(".txt") {
    ///             let content = String::from_utf8_lossy(&file.data);
    ///             println!("Text file content: {}", content);
    ///         }
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
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
///
/// # Examples
///
/// ```rust,no_run
/// use tako::extractors::multipart::{TakoTypedMultipart, UploadedFile};
/// use serde::Deserialize;
///
/// #[derive(Deserialize)]
/// struct ProfileForm {
///     name: String,
///     bio: String,
///     avatar: UploadedFile,
/// }
///
/// async fn update_profile(
///     TakoTypedMultipart { data: form, .. }: TakoTypedMultipart<'_, ProfileForm, UploadedFile>
/// ) -> Result<(), Box<dyn std::error::Error>> {
///     println!("Name: {}", form.name);
///     println!("Bio: {}", form.bio);
///     println!("Avatar saved to: {:?}", form.avatar.path);
///     Ok(())
/// }
/// ```
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

    /// Extracts a `TakoTypedMultipart` instance from an HTTP request.
    ///
    /// This function parses the multipart form data, deserializes text fields into
    /// a JSON-compatible structure, and processes file fields using the `FromMultipartField`
    /// trait. The result is a strongly-typed structure containing both text and file data.
    ///
    /// # Processing Logic
    ///
    /// 1. Validates Content-Type header and parses boundary
    /// 2. Iterates through all multipart fields
    /// 3. File fields (with filename) are processed using `F::from_field`
    /// 4. Text fields are collected as strings
    /// 5. All fields are combined into a JSON object
    /// 6. The JSON is deserialized into type `T`
    ///
    /// # Errors
    ///
    /// Returns `TypedMultipartError` if:
    /// - Content-Type header is missing or invalid
    /// - Boundary parsing fails
    /// - Field processing fails
    /// - Deserialization into type `T` fails
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, multipart::{TakoTypedMultipart, InMemoryFile}};
    /// use tako::types::Request;
    /// use serde::Deserialize;
    ///
    /// #[derive(Deserialize)]
    /// struct DocumentForm {
    ///     title: String,
    ///     content: String,
    ///     attachment: InMemoryFile,
    /// }
    ///
    /// async fn submit_document(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let TakoTypedMultipart { data: form, .. } =
    ///         TakoTypedMultipart::<DocumentForm, InMemoryFile>::from_request(&mut req).await?;
    ///
    ///     println!("Document: {}", form.title);
    ///     println!("Content length: {}", form.content.len());
    ///     println!("Attachment size: {} bytes", form.attachment.data.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
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
                    .ok_or_else(|| {
                        TypedMultipartError::FieldError("Field name missing".to_string())
                    })?
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
