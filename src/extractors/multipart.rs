use std::path::PathBuf;

use anyhow::{Context, Result};
use http::header::CONTENT_TYPE;
use http_body_util::BodyExt;
use multer::Multipart;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Map, Value};
use tokio::{fs::File, io::AsyncWriteExt};
use uuid::Uuid;

use crate::{
    extractors::{AsyncFromRequestMut, FromRequestMut},
    types::Request,
};

/// Wrapper around `multer::Multipart` to provide additional functionality.
pub struct TakoMultipart<'a>(pub Multipart<'a>);

impl<'a> TakoMultipart<'a> {
    /// Consumes the wrapper and returns the inner `Multipart` instance.
    #[inline]
    pub fn into_inner(self) -> Multipart<'a> {
        self.0
    }
}

impl<'a> FromRequestMut<'a> for TakoMultipart<'a> {
    /// Extracts a `TakoMultipart` instance from an HTTP request.
    ///
    /// This function checks for the `Content-Type` header, parses the boundary,
    /// and creates a `Multipart` instance from the request body.
    fn from_request(req: &'a mut Request) -> Result<Self> {
        let ct = req
            .headers()
            .get(CONTENT_TYPE)
            .context("Missing `Content-Type` header")?
            .to_str()
            .context("Invalid `Content-Type` header")?;

        let boundary = multer::parse_boundary(ct)
            .context("Request is not multipart/form-data or boundary is missing")?;
        let body_stream = req.body_mut().into_data_stream();

        Ok(Self(Multipart::new(body_stream, boundary)))
    }
}

/// Trait for types that can be constructed from a multipart field.
pub trait FromMultipartField: Serialize + Sized {
    /// Constructs an instance of the type from a `multer::Field`.
    fn from_field(field: multer::Field<'_>) -> impl Future<Output = Result<Self>>;
}

/// Represents a file uploaded to the server and saved to disk.
/// Represents a file uploaded to the server and stored in memory.
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
    async fn from_field(mut field: multer::Field<'_>) -> Result<Self> {
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
    async fn from_field(field: multer::Field<'_>) -> Result<Self> {
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

impl<'a, T, F> AsyncFromRequestMut<'a> for TakoTypedMultipart<'a, T, F>
where
    T: DeserializeOwned + 'static,
    F: FromMultipartField + serde::Serialize + 'static,
{
    /// Extracts a `TakoTypedMultipart` instance from an HTTP request.
    ///
    /// This function parses the multipart form data, deserializes text fields into
    /// a JSON-compatible structure, and processes file fields using the `FromMultipartField` trait.
    async fn from_request(req: &'a mut Request) -> Result<Self> {
        let ct = req
            .headers()
            .get(CONTENT_TYPE)
            .context("Content-Type is missing")?
            .to_str()
            .context("Content-Type is not UTF-8")?;
        let boundary =
            multer::parse_boundary(ct).context("Not multipart/form-data or boundary missing")?;

        let mut mp = Multipart::new(req.body_mut().into_data_stream(), boundary);
        let mut map = Map::<String, Value>::new();

        while let Some(field) = mp.next_field().await? {
            let name = field
                .name()
                .map(|s| s.to_owned())
                .context("field name missing")?;

            if field.file_name().is_some() {
                let file_val: F = F::from_field(field).await?;
                map.insert(name, serde_json::to_value(file_val)?);
            } else {
                let text = String::from_utf8(field.bytes().await?.to_vec())?;
                map.insert(name, Value::String(text));
            }
        }

        let data: T = serde_json::from_value(Value::Object(map))?;
        Ok(Self {
            data,
            _marker: core::marker::PhantomData,
        })
    }
}
