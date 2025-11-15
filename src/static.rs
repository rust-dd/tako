//! Static file serving utilities for web applications.
//!
//! This module provides functionality for serving static files and directories over HTTP.
//! It includes `ServeDir` for serving entire directories with optional fallback files,
//! and `ServeFile` for serving individual files. Both support automatic MIME type
//! detection, security path validation, and builder patterns for configuration.
//!
//! # Examples
//!
//! ```rust
//! use tako::r#static::{ServeDir, ServeFile};
//! use tako::types::Request;
//! use tako::body::TakoBody;
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Serve a directory with fallback
//! let serve_dir = ServeDir::builder("./public")
//!     .fallback("./public/index.html")
//!     .build();
//!
//! // Serve a single file
//! let serve_file = ServeFile::builder("./assets/logo.png").build();
//!
//! let request = Request::builder().body(TakoBody::empty())?;
//! let _response = serve_dir.handle(request).await;
//! # Ok(())
//! # }
//! ```

use std::path::{Path, PathBuf};

use http::StatusCode;
use tokio::fs;

use crate::{
  body::TakoBody,
  responder::Responder,
  types::{Request, Response},
};

/// Static directory server with configurable fallback handling.
pub struct ServeDir {
  base_dir: PathBuf,
  fallback: Option<PathBuf>,
}

/// Builder for configuring a `ServeDir` instance.
pub struct ServeDirBuilder {
  base_dir: PathBuf,
  fallback: Option<PathBuf>,
}

impl ServeDirBuilder {
  /// Creates a new builder with the specified base directory.
  pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
    Self {
      base_dir: base_dir.into(),
      fallback: None,
    }
  }

  /// Sets a fallback file to serve when requested files are not found.
  pub fn fallback<P: Into<PathBuf>>(mut self, fallback: P) -> Self {
    self.fallback = Some(fallback.into());
    self
  }

  /// Builds and returns the configured `ServeDir` instance.
  pub fn build(self) -> ServeDir {
    ServeDir {
      base_dir: self.base_dir,
      fallback: self.fallback,
    }
  }
}

impl ServeDir {
  /// Creates a new builder for configuring a `ServeDir`.
  pub fn builder<P: Into<PathBuf>>(base_dir: P) -> ServeDirBuilder {
    ServeDirBuilder::new(base_dir)
  }

  /// Sanitizes the requested path to prevent directory traversal attacks.
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

  /// Serves a file from the given path with appropriate MIME type.
  async fn serve_file(&self, file_path: &Path) -> Option<Response> {
    match fs::read(file_path).await {
      Ok(contents) => {
        let mime = mime_guess::from_path(file_path).first_or_octet_stream();
        Some(
          http::Response::builder()
            .status(StatusCode::OK)
            .header(http::header::CONTENT_TYPE, mime.to_string())
            .body(TakoBody::from(contents))
            .unwrap(),
        )
      }
      Err(_) => None,
    }
  }

  /// Handles an HTTP request to serve a static file from the directory.
  pub async fn handle(&self, req: Request) -> impl Responder {
    let path = req.uri().path();

    if let Some(file_path) = self.sanitize_path(path)
      && let Some(resp) = self.serve_file(&file_path).await
    {
      return resp;
    }

    if let Some(fallback) = &self.fallback
      && let Some(resp) = self.serve_file(fallback).await
    {
      return resp;
    }

    http::Response::builder()
      .status(StatusCode::NOT_FOUND)
      .body(TakoBody::from("File not found"))
      .unwrap()
  }
}

/// Static file server for serving individual files.
pub struct ServeFile {
  path: PathBuf,
}

/// Builder for configuring a `ServeFile` instance.
pub struct ServeFileBuilder {
  path: PathBuf,
}

impl ServeFileBuilder {
  /// Creates a new builder with the specified file path.
  pub fn new<P: Into<PathBuf>>(path: P) -> Self {
    Self { path: path.into() }
  }

  /// Builds and returns the configured `ServeFile` instance.
  pub fn build(self) -> ServeFile {
    ServeFile { path: self.path }
  }
}

impl ServeFile {
  /// Creates a new builder for configuring a `ServeFile`.
  pub fn builder<P: Into<PathBuf>>(path: P) -> ServeFileBuilder {
    ServeFileBuilder::new(path)
  }

  /// Serves the configured file with appropriate MIME type.
  async fn serve_file(&self) -> Option<Response> {
    match fs::read(&self.path).await {
      Ok(contents) => {
        let mime = mime_guess::from_path(&self.path).first_or_octet_stream();
        Some(
          http::Response::builder()
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
  pub async fn handle(&self, _req: Request) -> impl Responder {
    if let Some(resp) = self.serve_file().await {
      resp
    } else {
      http::Response::builder()
        .status(StatusCode::NOT_FOUND)
        .body(TakoBody::from("File not found"))
        .unwrap()
    }
  }
}
