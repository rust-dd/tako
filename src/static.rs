use std::path::{Path, PathBuf};

use http::StatusCode;
use tokio::fs;

use crate::{
    body::TakoBody,
    responder::Responder,
    types::{Request, Response},
};

/// A struct representing a directory to serve static files from.
///
/// `ServeDir` allows serving files from a base directory and optionally
/// falling back to a specific file if the requested file is not found.
pub struct ServeDir {
    base_dir: PathBuf,
    fallback: Option<PathBuf>,
}

/// A builder for creating a `ServeDir` instance.
///
/// This struct provides a fluent API to configure the base directory
/// and an optional fallback file for the `ServeDir`.
pub struct ServeDirBuilder {
    base_dir: PathBuf,
    fallback: Option<PathBuf>,
}

impl ServeDirBuilder {
    /// Creates a new `ServeDirBuilder` with the specified base directory.
    ///
    /// # Arguments
    ///
    /// * `base_dir` - The base directory to serve files from.
    pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.into(),
            fallback: None,
        }
    }

    /// Sets a fallback file to serve when a requested file is not found.
    ///
    /// # Arguments
    ///
    /// * `fallback` - The path to the fallback file.
    pub fn fallback<P: Into<PathBuf>>(mut self, fallback: P) -> Self {
        self.fallback = Some(fallback.into());
        self
    }

    /// Builds and returns a `ServeDir` instance with the configured options.
    pub fn build(self) -> ServeDir {
        ServeDir {
            base_dir: self.base_dir,
            fallback: self.fallback,
        }
    }
}

impl ServeDir {
    /// Creates a new `ServeDirBuilder` for configuring a `ServeDir`.
    ///
    /// # Arguments
    ///
    /// * `base_dir` - The base directory to serve files from.
    pub fn builder<P: Into<PathBuf>>(base_dir: P) -> ServeDirBuilder {
        ServeDirBuilder::new(base_dir)
    }

    /// Sanitizes the requested path to ensure it is within the base directory.
    ///
    /// This function resolves the requested path relative to the base directory
    /// and ensures it does not escape the base directory.
    ///
    /// # Arguments
    ///
    /// * `req_path` - The requested file path from the HTTP request.
    ///
    /// # Returns
    ///
    /// An optional `PathBuf` representing the sanitized path.
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

    /// Serves a file from the given path.
    ///
    /// This function reads the file contents and constructs an HTTP response
    /// with the appropriate MIME type.
    ///
    /// # Arguments
    ///
    /// * `file_path` - The path to the file to be served.
    ///
    /// # Returns
    ///
    /// An optional `Response` containing the file contents.
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

    /// Handles an HTTP request to serve a static file.
    ///
    /// This function attempts to serve the requested file. If the file is not found,
    /// it optionally serves a fallback file or returns a 404 response.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request object.
    ///
    /// # Returns
    ///
    /// A response object implementing the `Responder` trait.
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

/// A struct representing a single static file to be served.
///
/// `ServeFile` is used to serve a specific file in response to HTTP requests.
pub struct ServeFile {
    path: PathBuf,
}

/// A builder for creating a `ServeFile` instance.
///
/// This struct provides a fluent API to configure the file to be served.
pub struct ServeFileBuilder {
    path: PathBuf,
}

impl ServeFileBuilder {
    /// Creates a new `ServeFileBuilder` with the specified file path.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the file to be served.
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self { path: path.into() }
    }

    /// Builds and returns a `ServeFile` instance with the configured file path.
    pub fn build(self) -> ServeFile {
        ServeFile { path: self.path }
    }
}

impl ServeFile {
    /// Creates a new `ServeFileBuilder` for configuring a `ServeFile`.
    ///
    /// # Arguments
    ///
    /// * `path` - The path to the file to be served.
    pub fn builder<P: Into<PathBuf>>(path: P) -> ServeFileBuilder {
        ServeFileBuilder::new(path)
    }

    /// Serves the configured file.
    ///
    /// This function reads the file contents and constructs an HTTP response
    /// with the appropriate MIME type.
    ///
    /// # Returns
    ///
    /// An optional `Response` containing the file contents.
    async fn serve_file(&self) -> Option<Response> {
        match fs::read(&self.path).await {
            Ok(contents) => {
                let mime = mime_guess::from_path(&self.path).first_or_octet_stream();
                Some(
                    hyper::Response::builder()
                        .status(StatusCode::OK)
                        .header(http::header::CONTENT_TYPE, mime.to_string())
                        .body(TakoBody::from(contents))
                        .unwrap(),
                )
            }
            Err(_) => None,
        }
    }

    /// Handles an HTTP request to serve the configured static file.
    ///
    /// This function serves the file if it exists, or returns a 404 response
    /// if the file is not found.
    ///
    /// # Arguments
    ///
    /// * `_req` - The HTTP request object (unused in this implementation).
    ///
    /// # Returns
    ///
    /// A response object implementing the `Responder` trait.
    pub async fn handle(&self, _req: Request) -> impl Responder {
        if let Some(resp) = self.serve_file().await {
            resp
        } else {
            hyper::Response::builder()
                .status(StatusCode::NOT_FOUND)
                .body(TakoBody::from("File not found"))
                .unwrap()
        }
    }
}
