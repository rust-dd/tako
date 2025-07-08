use std::path::{Path, PathBuf};

use http::StatusCode;
use tokio::fs;

use crate::{
    body::TakoBody,
    responder::Responder,
    types::{Request, Response},
};

pub struct ServeDir {
    pub(crate) base_dir: PathBuf,
    pub(crate) fallback: Option<PathBuf>,
}

pub struct ServeDirBuilder {
    base_dir: PathBuf,
    fallback: Option<PathBuf>,
}

impl ServeDirBuilder {
    pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.into(),
            fallback: None,
        }
    }

    pub fn fallback<P: Into<PathBuf>>(mut self, fallback: P) -> Self {
        self.fallback = Some(fallback.into());
        self
    }

    pub fn build(self) -> ServeDir {
        ServeDir {
            base_dir: self.base_dir,
            fallback: self.fallback,
        }
    }
}

impl ServeDir {
    pub fn builder<P: Into<PathBuf>>(base_dir: P) -> ServeDirBuilder {
        ServeDirBuilder::new(base_dir)
    }

    fn sanitize_path(&self, req_path: &str) -> Option<PathBuf> {
        let rel_path = req_path.trim_start_matches('/');
        let joined = self.base_dir.join(rel_path);
        let canonical = joined.canonicalize().ok()?;
        if canonical.starts_with(self.base_dir.canonicalize().ok()?) {
            Some(canonical)
        } else {
            None
        }
    }

    async fn serve_file(&self, file_path: &Path) -> Option<Response> {
        match fs::read(file_path).await {
            Ok(contents) => {
                let mime = mime_guess::from_path(file_path).first_or_octet_stream();
                Some(
                    hyper::Response::builder()
                        .status(StatusCode::OK)
                        .header(hyper::header::CONTENT_TYPE, mime.to_string())
                        .body(TakoBody::from(contents))
                        .unwrap(),
                )
            }
            Err(_) => None,
        }
    }

    pub async fn handle(&self, req: Request) -> impl Responder {
        let path = req.uri().path();

        if let Some(file_path) = self.sanitize_path(path) {
            if let Some(resp) = self.serve_file(&file_path).await {
                return resp;
            }
        }

        if let Some(fallback) = &self.fallback {
            if let Some(resp) = self.serve_file(fallback).await {
                return resp;
            }
        }

        hyper::Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(TakoBody::from("File not found"))
            .unwrap()
    }
}
