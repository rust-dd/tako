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
#[derive(Debug)]
pub enum MultipartError {
    MissingContentType,
    InvalidContentType,
    InvalidUtf8,
    BoundaryParseError(String),
}

impl Responder for MultipartError {
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
    MissingContentType,
    InvalidContentType,
    InvalidUtf8,
    BoundaryParseError(String),
    FieldError(String),
    DeserializationError(String),
    IoError(String),
}

impl Responder for TypedMultipartError {
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

    /// Extracts a `TakoMultipart` instance from an HTTP request.
    ///
    /// This function checks for the `Content-Type` header, parses the boundary,
    /// and creates a `Multipart` instance from the request body.
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_multipart(req))
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
pub trait FromMultipartField: Serialize + Sized {
    /// Constructs an instance of the type from a `multer::Field`.
    fn from_field(
        field: multer::Field<'_>,
    ) -> impl std::future::Future<Output = anyhow::Result<Self>> + Send;
}

/// Represents a file uploaded to the server and saved to disk.
#[derive(Debug, Serialize, Deserialize)]
pub struct UploadedFile {
    pub file_name: Option<String>,    // Original file name, if provided.
    pub content_type: Option<String>, // MIME type of the file.
    pub path: PathBuf,                // Path to the saved file on disk.
    pub size: u64,                    // Size of the file in bytes.
}

impl FromMultipartField for UploadedFile {
    /// Creates an `UploadedFile` instance from a multipart field.
    ///
    /// The file is saved to a temporary directory, and its metadata is stored in the struct.
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
    pub file_name: Option<String>,    // Original file name, if provided.
    pub content_type: Option<String>, // MIME type of the file.
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>, // File content stored as a byte array.
}

impl FromMultipartField for InMemoryFile {
    /// Creates an `InMemoryFile` instance from a multipart field.
    ///
    /// The file content is stored in memory as a byte array.
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
/// structure, combining both file and text fields.
pub struct TakoTypedMultipart<'a, T, F> {
    pub data: T, // Deserialized data from the multipart request.
    _marker: core::marker::PhantomData<&'a F>, // Marker for the field type.
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
    /// a JSON-compatible structure, and processes file fields using the `FromMultipartField` trait.
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
