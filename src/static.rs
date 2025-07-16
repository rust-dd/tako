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
///
/// `ServeDir` serves files from a base directory and optionally falls back to a
/// specific file when the requested file is not found. This is useful for serving
/// static assets in web applications, especially single-page applications that
/// need to serve an index.html file for client-side routing.
pub struct ServeDir {
    base_dir: PathBuf,
    fallback: Option<PathBuf>,
}

/// Builder for configuring a `ServeDir` instance.
///
/// Provides a fluent API for setting up directory serving with optional fallback
/// file configuration. The builder pattern ensures all required parameters are
/// provided while keeping optional parameters clearly separated.
pub struct ServeDirBuilder {
    base_dir: PathBuf,
    fallback: Option<PathBuf>,
}

impl ServeDirBuilder {
    /// Creates a new builder with the specified base directory.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeDirBuilder;
    ///
    /// let builder = ServeDirBuilder::new("./public");
    /// let serve_dir = builder.build();
    /// ```
    pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
        Self {
            base_dir: base_dir.into(),
            fallback: None,
        }
    }

    /// Sets a fallback file to serve when requested files are not found.
    ///
    /// The fallback file is typically used for single-page applications where
    /// all routes should serve the main HTML file for client-side routing.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeDirBuilder;
    ///
    /// let serve_dir = ServeDirBuilder::new("./public")
    ///     .fallback("./public/index.html")
    ///     .build();
    /// ```
    pub fn fallback<P: Into<PathBuf>>(mut self, fallback: P) -> Self {
        self.fallback = Some(fallback.into());
        self
    }

    /// Builds and returns the configured `ServeDir` instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeDirBuilder;
    ///
    /// let serve_dir = ServeDirBuilder::new("./assets")
    ///     .fallback("./assets/404.html")
    ///     .build();
    /// ```
    pub fn build(self) -> ServeDir {
        ServeDir {
            base_dir: self.base_dir,
            fallback: self.fallback,
        }
    }
}

impl ServeDir {
    /// Creates a new builder for configuring a `ServeDir`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeDir;
    ///
    /// let serve_dir = ServeDir::builder("./static")
    ///     .fallback("./static/index.html")
    ///     .build();
    /// ```
    pub fn builder<P: Into<PathBuf>>(base_dir: P) -> ServeDirBuilder {
        ServeDirBuilder::new(base_dir)
    }

    /// Sanitizes the requested path to prevent directory traversal attacks.
    ///
    /// This function resolves the requested path relative to the base directory
    /// and ensures it doesn't escape outside the allowed directory tree using
    /// canonical path resolution and prefix checking.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeDir;
    /// use std::path::PathBuf;
    ///
    /// let serve_dir = ServeDir::builder("./public").build();
    ///
    /// // Valid paths within base directory
    /// let result = serve_dir.sanitize_path("/index.html");
    ///
    /// // Invalid paths attempting traversal return None
    /// let result = serve_dir.sanitize_path("/../etc/passwd");
    /// assert!(result.is_none());
    /// ```
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
    ///
    /// Reads the file contents and constructs an HTTP response with the correct
    /// Content-Type header based on the file extension. Returns None if the file
    /// cannot be read or doesn't exist.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::r#static::ServeDir;
    /// use std::path::Path;
    ///
    /// # async fn example() {
    /// let serve_dir = ServeDir::builder("./public").build();
    /// let path = Path::new("./public/style.css");
    ///
    /// if let Some(response) = serve_dir.serve_file(path).await {
    ///     println!("File served successfully");
    /// }
    /// # }
    /// ```
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

    /// Handles an HTTP request to serve a static file from the directory.
    ///
    /// Attempts to serve the requested file from the base directory. If the file
    /// is not found and a fallback is configured, serves the fallback file instead.
    /// Returns a 404 response if neither the requested file nor fallback can be served.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::r#static::ServeDir;
    /// use tako::types::Request;
    /// use tako::body::TakoBody;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let serve_dir = ServeDir::builder("./public")
    ///     .fallback("./public/index.html")
    ///     .build();
    ///
    /// let request = Request::builder()
    ///     .uri("/assets/style.css")
    ///     .body(TakoBody::empty())?;
    ///
    /// let response = serve_dir.handle(request).await;
    /// println!("Response status: {}", response.status());
    /// # Ok(())
    /// # }
    /// ```
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

/// Static file server for serving individual files.
///
/// `ServeFile` is used to serve a specific file in response to HTTP requests.
/// This is useful for serving individual assets like favicons, robots.txt,
/// or any other static file that should be served from a fixed location.
pub struct ServeFile {
    path: PathBuf,
}

/// Builder for configuring a `ServeFile` instance.
///
/// Provides a simple builder interface for creating file servers. While currently
/// minimal, the builder pattern allows for future extensibility without breaking
/// existing code.
pub struct ServeFileBuilder {
    path: PathBuf,
}

impl ServeFileBuilder {
    /// Creates a new builder with the specified file path.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeFileBuilder;
    ///
    /// let builder = ServeFileBuilder::new("./assets/favicon.ico");
    /// let serve_file = builder.build();
    /// ```
    pub fn new<P: Into<PathBuf>>(path: P) -> Self {
        Self { path: path.into() }
    }

    /// Builds and returns the configured `ServeFile` instance.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeFileBuilder;
    ///
    /// let serve_file = ServeFileBuilder::new("./robots.txt").build();
    /// ```
    pub fn build(self) -> ServeFile {
        ServeFile { path: self.path }
    }
}

impl ServeFile {
    /// Creates a new builder for configuring a `ServeFile`.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::r#static::ServeFile;
    ///
    /// let serve_file = ServeFile::builder("./assets/logo.png").build();
    /// ```
    pub fn builder<P: Into<PathBuf>>(path: P) -> ServeFileBuilder {
        ServeFileBuilder::new(path)
    }

    /// Serves the configured file with appropriate MIME type.
    ///
    /// Reads the file contents and constructs an HTTP response with the correct
    /// Content-Type header based on the file extension. Returns None if the file
    /// cannot be read or doesn't exist.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::r#static::ServeFile;
    ///
    /// # async fn example() {
    /// let serve_file = ServeFile::builder("./favicon.ico").build();
    ///
    /// if let Some(response) = serve_file.serve_file().await {
    ///     println!("File served successfully");
    /// }
    /// # }
    /// ```
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
    /// Serves the file if it exists, or returns a 404 response if the file
    /// cannot be found or read. The request parameter is not used in the
    /// current implementation since the file path is predetermined.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::r#static::ServeFile;
    /// use tako::types::Request;
    /// use tako::body::TakoBody;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let serve_file = ServeFile::builder("./robots.txt").build();
    ///
    /// let request = Request::builder()
    ///     .uri("/robots.txt")
    ///     .body(TakoBody::empty())?;
    ///
    /// let response = serve_file.handle(request).await;
    /// println!("Response status: {}", response.status());
    /// # Ok(())
    /// # }
    /// ```
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
