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

pub struct TakoMultipart<'a>(pub Multipart<'a>);

impl<'a> TakoMultipart<'a> {
    #[inline]
    pub fn into_inner(self) -> Multipart<'a> {
        self.0
    }
}

impl<'a> FromRequestMut<'a> for TakoMultipart<'a> {
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

pub trait FromMultipartField: Serialize + Sized {
    fn from_field(field: multer::Field<'_>) -> impl Future<Output = Result<Self>>;
}

#[derive(Debug, Serialize, Deserialize)]
pub struct UploadedFile {
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    pub path: PathBuf,
    pub size: u64,
}

impl FromMultipartField for UploadedFile {
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
    pub file_name: Option<String>,
    pub content_type: Option<String>,
    #[serde(with = "serde_bytes")]
    pub data: Vec<u8>,
}

impl FromMultipartField for InMemoryFile {
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

pub struct TakoTypedMultipart<'a, T, F> {
    pub data: T,
    _marker: core::marker::PhantomData<&'a F>,
}

impl<'a, T, F> AsyncFromRequestMut<'a> for TakoTypedMultipart<'a, T, F>
where
    T: DeserializeOwned + 'static,
    F: FromMultipartField + serde::Serialize + 'static,
{
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
