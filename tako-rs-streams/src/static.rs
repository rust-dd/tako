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
use tako_rs_core::body::TakoBody;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;
#[cfg(not(feature = "compio"))]
use tokio::fs;
#[cfg(not(feature = "compio"))]
use tokio::io::AsyncReadExt;

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

  /// Verifies a sidecar path (`<file>.br` / `<file>.gz`) canonicalizes to
  /// somewhere inside the base directory before we hand it to the open
  /// pipeline. The original base-prefix check only covered `file_path`; a
  /// symlinked sidecar could otherwise escape outside the base.
  fn canonical_within_base(&self, p: &Path) -> Option<PathBuf> {
    let canonical = p.canonicalize().ok()?;
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

  fn precompressed_variant(
    &self,
    file_path: &Path,
    headers: &http::HeaderMap,
  ) -> Option<(PathBuf, &'static str)> {
    if self.precompressed.brotli && Self::accepts(headers, "br") {
      let mut p = file_path.as_os_str().to_owned();
      p.push(".br");
      let p = PathBuf::from(p);
      if let Some(canonical) = self.canonical_within_base(&p) {
        return Some((canonical, "br"));
      }
    }
    if self.precompressed.gzip && Self::accepts(headers, "gzip") {
      let mut p = file_path.as_os_str().to_owned();
      p.push(".gz");
      let p = PathBuf::from(p);
      if let Some(canonical) = self.canonical_within_base(&p) {
        return Some((canonical, "gzip"));
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
        if !cand.is_file() {
          continue;
        }
        // Critical: re-run the canonical-within-base check on the index
        // file. `cand.is_file()` follows symlinks, so an in-base
        // `index.html` that links out of the base (e.g. to `/etc/passwd`)
        // would otherwise be served — exactly the escape vector the
        // sidecar fix (C8) closed on the `.br`/`.gz` path. Cold path —
        // only fires when the request targets a directory.
        if let Some(canonical) = self.canonical_within_base(&cand) {
          chosen = Some(canonical);
          break;
        }
      }
      chosen?
    } else {
      file_path
    };

    if let Some((compressed, encoding)) = self.precompressed_variant(&target, headers) {
      if let Some(resp) = Self::serve_file_with_encoding(&compressed, &target, encoding).await {
        return Some((resp, encoding));
      }
      // Sidecar read failed (deleted between resolve and open, permission
      // glitch, etc.) — fall through to the identity file instead of
      // 404-ing the whole request.
      tracing::debug!(
        target = %target.display(),
        encoding,
        "precompressed sidecar read failed, falling back to identity"
      );
    }

    Some((Self::serve_file(&target).await?, "identity"))
  }

  /// Open the file via a single `File::open` (resolves symlinks exactly once),
  /// verify the result is a regular file via the open FD's metadata (defense
  /// in depth against directory/special-file confusion), then read. This
  /// replaces the prior `fs::read` pattern which would re-resolve the path
  /// after the caller had already canonicalized it.
  #[cfg(not(feature = "compio"))]
  async fn open_and_read_regular(path: &Path) -> Option<Vec<u8>> {
    let mut file = fs::File::open(path).await.ok()?;
    let meta = file.metadata().await.ok()?;
    if !meta.is_file() {
      return None;
    }
    let mut contents = Vec::with_capacity(meta.len() as usize);
    file.read_to_end(&mut contents).await.ok()?;
    Some(contents)
  }

  #[cfg(feature = "compio")]
  async fn open_and_read_regular(path: &Path) -> Option<Vec<u8>> {
    // compio uses positional read with owned buffers; the high-level
    // `fs::read` already wraps open + read + metadata. The canonical-prefix
    // check is performed by the caller before we get here.
    //
    // STR-8: this loads the whole file into RAM, unlike the tokio path
    // that streams via ReaderStream. Operators serving large static
    // assets under compio must cap file sizes at the route level (or
    // switch to the tokio backend) — a multi-GB asset will land as a
    // single `Vec<u8>` per request. Replacing with a chunked
    // `compio::fs::File::read_at` loop is a 2.x deferral; the type
    // surface change ripples through `StaticDir`'s body shape.
    let meta = fs::metadata(path).await.ok()?;
    if !meta.is_file() {
      return None;
    }
    fs::read(path).await.ok()
  }

  async fn serve_file(file_path: &Path) -> Option<Response> {
    let contents = Self::open_and_read_regular(file_path).await?;
    let mime = mime_guess::from_path(file_path).first_or_octet_stream();
    Some(
      http::Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.to_string())
        .body(TakoBody::from(contents))
        .unwrap(),
    )
  }

  async fn serve_file_with_encoding(
    compressed: &Path,
    original: &Path,
    encoding: &'static str,
  ) -> Option<Response> {
    let contents = Self::open_and_read_regular(compressed).await?;
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
  ///
  /// The request itself is **ignored** — `ServeFile` always serves the file
  /// configured on the builder, regardless of `req.uri()`. Mount this
  /// handler on a single specific route (e.g. `/manifest.json`), not on a
  /// catch-all glob, otherwise every URL under that glob will return the
  /// same file. Use [`ServeDir`] when you want path-aware static serving.
  pub async fn handle(&self, _req: Request) -> impl Responder {
    if let Some(resp) = self.serve_file().await {
      resp
    } else {
      let mut resp = http::Response::new(TakoBody::from("File not found"));
      *resp.status_mut() = StatusCode::NOT_FOUND;
      resp
    }
  }
}
