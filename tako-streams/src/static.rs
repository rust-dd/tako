//! Static file serving utilities for web applications.
//!
//! `ServeDir` serves files from a directory tree with index resolution,
//! precompressed-asset preference (`*.br` / `*.gz`), an SPA fallback rewrite,
//! and a canonicalize + prefix-check guard against path traversal.
//!
//! `ServeFile` serves a single file.

use std::path::Path;
use std::path::PathBuf;

#[cfg(feature = "compio")]
use compio::fs;
use http::StatusCode;
use http::header;
use tako_core::body::TakoBody;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;
#[cfg(not(feature = "compio"))]
use tokio::fs;

/// Static directory server with configurable fallback handling.
#[doc(alias = "static")]
#[doc(alias = "serve_dir")]
pub struct ServeDir {
  base_dir: PathBuf,
  fallback: Option<PathBuf>,
  index_files: Vec<String>,
  precompressed: PrecompressedPolicy,
  sanitized_base: Option<PathBuf>,
}

/// Which precompressed sidecar files (if any) `ServeDir` should prefer when
/// the client advertises support via `Accept-Encoding`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrecompressedPolicy {
  /// Serve `<file>.br` when the client accepts `br`.
  pub brotli: bool,
  /// Serve `<file>.gz` when the client accepts `gzip`.
  pub gzip: bool,
}

impl PrecompressedPolicy {
  /// Both `br` and `gzip` enabled.
  pub const fn both() -> Self {
    Self {
      brotli: true,
      gzip: true,
    }
  }

  /// `br` only.
  pub const fn brotli_only() -> Self {
    Self {
      brotli: true,
      gzip: false,
    }
  }

  /// `gzip` only.
  pub const fn gzip_only() -> Self {
    Self {
      brotli: false,
      gzip: true,
    }
  }
}

/// Builder for configuring a `ServeDir` instance.
#[must_use]
pub struct ServeDirBuilder {
  base_dir: PathBuf,
  fallback: Option<PathBuf>,
  index_files: Vec<String>,
  precompressed: PrecompressedPolicy,
}

impl ServeDirBuilder {
  /// Creates a new builder with the specified base directory.
  #[inline]
  pub fn new<P: Into<PathBuf>>(base_dir: P) -> Self {
    Self {
      base_dir: base_dir.into(),
      fallback: None,
      index_files: vec!["index.html".into(), "index.htm".into()],
      precompressed: PrecompressedPolicy::default(),
    }
  }

  /// Sets a fallback file to serve when requested files are not found.
  #[inline]
  pub fn fallback<P: Into<PathBuf>>(mut self, fallback: P) -> Self {
    self.fallback = Some(fallback.into());
    self
  }

  /// Replace the index resolution priority list (defaults to
  /// `["index.html", "index.htm"]`).
  #[inline]
  pub fn index_files<I, S>(mut self, names: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.index_files = names.into_iter().map(Into::into).collect();
    self
  }

  /// Configure preference for precompressed sidecar files.
  #[inline]
  pub fn precompressed(mut self, policy: PrecompressedPolicy) -> Self {
    self.precompressed = policy;
    self
  }

  /// Builds and returns the configured `ServeDir` instance.
  #[inline]
  pub fn build(self) -> ServeDir {
    let sanitized_base = self.base_dir.canonicalize().ok();
    ServeDir {
      base_dir: self.base_dir,
      fallback: self.fallback,
      index_files: self.index_files,
      precompressed: self.precompressed,
      sanitized_base,
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
    // Refuse explicit `..` traversal segments before touching the FS.
    if rel_path
      .split(['/', '\\'])
      .any(|seg| seg == ".." || seg == ".")
    {
      return None;
    }
    let joined = self.base_dir.join(rel_path);
    let canonical = joined.canonicalize().ok()?;
    let base = self
      .sanitized_base
      .clone()
      .or_else(|| self.base_dir.canonicalize().ok())?;
    if canonical.starts_with(&base) {
      Some(canonical)
    } else {
      None
    }
  }

  fn accepts(headers: &http::HeaderMap, encoding: &str) -> bool {
    let Some(v) = headers
      .get(header::ACCEPT_ENCODING)
      .and_then(|v| v.to_str().ok())
    else {
      return false;
    };
    for part in v.split(',') {
      let part = part.trim();
      // Strip any q-value parameter; reject q=0 explicitly.
      let mut name_q = part.split(';');
      let name = name_q.next().unwrap_or("").trim();
      let q_zero = name_q.any(|p| p.trim().strip_prefix("q=").is_some_and(|q| q.trim() == "0"));
      if q_zero {
        continue;
      }
      if name.eq_ignore_ascii_case(encoding) || name == "*" {
        return true;
      }
    }
    false
  }

  fn precompressed_variant(
    &self,
    file_path: &Path,
    headers: &http::HeaderMap,
  ) -> Option<(PathBuf, &'static str)> {
    if self.precompressed.brotli && Self::accepts(headers, "br") {
      let mut p = file_path.as_os_str().to_owned();
      p.push(".br");
      let p = PathBuf::from(p);
      if p.is_file() {
        return Some((p, "br"));
      }
    }
    if self.precompressed.gzip && Self::accepts(headers, "gzip") {
      let mut p = file_path.as_os_str().to_owned();
      p.push(".gz");
      let p = PathBuf::from(p);
      if p.is_file() {
        return Some((p, "gzip"));
      }
    }
    None
  }

  async fn resolve_existing(
    &self,
    file_path: PathBuf,
    headers: &http::HeaderMap,
  ) -> Option<(Response, &'static str)> {
    // Index resolution if pointing at a directory.
    let target = if file_path.is_dir() {
      let mut chosen: Option<PathBuf> = None;
      for idx in &self.index_files {
        let cand = file_path.join(idx);
        if cand.is_file() {
          chosen = Some(cand);
          break;
        }
      }
      chosen?
    } else {
      file_path
    };

    if let Some((compressed, encoding)) = self.precompressed_variant(&target, headers) {
      return Some((
        Self::serve_file_with_encoding(&compressed, &target, encoding).await?,
        encoding,
      ));
    }

    Some((Self::serve_file(&target).await?, "identity"))
  }

  async fn serve_file(file_path: &Path) -> Option<Response> {
    match fs::read(file_path).await {
      Ok(contents) => {
        let mime = mime_guess::from_path(file_path).first_or_octet_stream();
        Some(
          http::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.to_string())
            .body(TakoBody::from(contents))
            .unwrap(),
        )
      }
      Err(_) => None,
    }
  }

  async fn serve_file_with_encoding(
    compressed: &Path,
    original: &Path,
    encoding: &'static str,
  ) -> Option<Response> {
    match fs::read(compressed).await {
      Ok(contents) => {
        let mime = mime_guess::from_path(original).first_or_octet_stream();
        Some(
          http::Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, mime.to_string())
            .header(header::CONTENT_ENCODING, encoding)
            .header(header::VARY, "Accept-Encoding")
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
    let headers = req.headers().clone();

    if let Some(file_path) = self.sanitize_path(path)
      && let Some((resp, _enc)) = self.resolve_existing(file_path, &headers).await
    {
      return resp;
    }

    if let Some(fallback) = &self.fallback
      && let Some((resp, _)) = self.resolve_existing(fallback.clone(), &headers).await
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
#[doc(alias = "serve_file")]
pub struct ServeFile {
  path: PathBuf,
}

/// Builder for configuring a `ServeFile` instance.
#[must_use]
pub struct ServeFileBuilder {
  path: PathBuf,
}

impl ServeFileBuilder {
  /// Creates a new builder with the specified file path.
  #[inline]
  pub fn new<P: Into<PathBuf>>(path: P) -> Self {
    Self { path: path.into() }
  }

  /// Builds and returns the configured `ServeFile` instance.
  #[inline]
  #[must_use]
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
