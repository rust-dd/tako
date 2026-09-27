use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;

use http::HeaderValue;

/// Static directory with streaming, cache validators, single byte ranges, and SPA fallback.
///
/// Dotfiles are denied by default. Keep the served tree read-only to untrusted
/// processes: canonical path checks cannot prevent concurrent filesystem replacement.
#[derive(Clone)]
#[doc(alias = "static")]
#[doc(alias = "serve_dir")]
pub struct ServeDir {
  pub(crate) config: Arc<DirectoryConfig>,
}

pub(crate) struct DirectoryConfig {
  pub(crate) base: PathBuf,
  pub(crate) fallback: Option<PathBuf>,
  pub(crate) index_files: Vec<String>,
  pub(crate) precompressed: PrecompressedPolicy,
  pub(crate) allow_dotfiles: bool,
  pub(crate) cache_control: Option<HeaderValue>,
}

/// Precompressed sidecars to negotiate through `Accept-Encoding`.
#[derive(Debug, Clone, Copy, Default)]
pub struct PrecompressedPolicy {
  /// Enable `<file>.br`.
  pub brotli: bool,
  /// Enable `<file>.gz`.
  pub gzip: bool,
}

impl PrecompressedPolicy {
  /// Enable Brotli and gzip.
  pub const fn both() -> Self {
    Self {
      brotli: true,
      gzip: true,
    }
  }
  /// Enable Brotli.
  pub const fn brotli_only() -> Self {
    Self {
      brotli: true,
      gzip: false,
    }
  }
  /// Enable gzip.
  pub const fn gzip_only() -> Self {
    Self {
      brotli: false,
      gzip: true,
    }
  }
}

/// Configure a static directory.
#[must_use]
pub struct ServeDirBuilder {
  config: DirectoryConfig,
}

impl ServeDirBuilder {
  /// Set the root directory.
  pub fn new(base: impl Into<PathBuf>) -> Self {
    Self {
      config: DirectoryConfig {
        base: base.into(),
        fallback: None,
        index_files: vec!["index.html".into(), "index.htm".into()],
        precompressed: PrecompressedPolicy::default(),
        allow_dotfiles: false,
        cache_control: None,
      },
    }
  }

  /// Serve this trusted file for missing paths. It may be outside the root.
  ///
  /// Invalid traversal and denied dotfile requests never use the fallback.
  pub fn fallback(mut self, path: impl Into<PathBuf>) -> Self {
    self.config.fallback = Some(path.into());
    self
  }

  /// Replace the index filename priority list.
  pub fn index_files<I, S>(mut self, names: I) -> Self
  where
    I: IntoIterator<Item = S>,
    S: Into<String>,
  {
    self.config.index_files = names.into_iter().map(Into::into).collect();
    self
  }

  /// Enable precompressed sidecar negotiation.
  pub fn precompressed(mut self, policy: PrecompressedPolicy) -> Self {
    self.config.precompressed = policy;
    self
  }

  /// Allow dotfile path components, including `.well-known`. Defaults to false.
  pub fn allow_dotfiles(mut self, allow: bool) -> Self {
    self.config.allow_dotfiles = allow;
    self
  }

  /// Set Cache-Control on successful and not-modified responses.
  pub fn cache_control(mut self, value: HeaderValue) -> Self {
    self.config.cache_control = Some(value);
    self
  }

  /// Finish configuration. Filesystem work happens on a blocking worker per request.
  pub fn build(self) -> ServeDir {
    ServeDir {
      config: Arc::new(self.config),
    }
  }
}

impl ServeDir {
  /// Configure a static directory.
  pub fn builder(base: impl Into<PathBuf>) -> ServeDirBuilder {
    ServeDirBuilder::new(base)
  }
}

impl DirectoryConfig {
  pub(crate) fn valid_relative(&self, path: &str) -> bool {
    !path.contains(['\0', '\\'])
      && path.split('/').all(|segment| {
        segment != "." && segment != ".." && (self.allow_dotfiles || !segment.starts_with('.'))
      })
      && Path::new(path)
        .components()
        .all(|component| matches!(component, std::path::Component::Normal(_)))
  }

  pub(crate) fn within_base(&self, path: &Path, base: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().ok()?;
    let relative = canonical.strip_prefix(base).ok()?;
    if !self.allow_dotfiles
      && relative
        .components()
        .any(|part| part.as_os_str().to_string_lossy().starts_with('.'))
    {
      return None;
    }
    Some(canonical)
  }
}
