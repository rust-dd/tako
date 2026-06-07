use std::path::Path;
use std::path::PathBuf;

use http::header;

/// Static directory server with configurable fallback handling.
#[doc(alias = "static")]
#[doc(alias = "serve_dir")]
pub struct ServeDir {
  pub(crate) base_dir: PathBuf,
  pub(crate) fallback: Option<PathBuf>,
  pub(crate) index_files: Vec<String>,
  pub(crate) precompressed: PrecompressedPolicy,
  pub(crate) sanitized_base: Option<PathBuf>,
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
  pub(crate) fn sanitize_path(&self, req_path: &str) -> Option<PathBuf> {
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
  pub(crate) fn canonical_within_base(&self, p: &Path) -> Option<PathBuf> {
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

  pub(crate) fn precompressed_variant(
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
}
