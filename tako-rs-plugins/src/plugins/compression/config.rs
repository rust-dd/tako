//! Compression configuration: content-type policy and runtime settings.

use super::encoding::Encoding;

/// Content-type matching policy.
#[derive(Clone, Default)]
pub enum ContentTypePolicy {
  /// Default heuristic: text/*, anything containing `json`, `javascript`, `xml`.
  #[default]
  Default,
  /// Exact MIME types (case-insensitive). E.g. `["application/json", "text/html"]`.
  Exact(Vec<String>),
  /// MIME prefixes (case-insensitive). E.g. `["text/", "application/x-json-"]`.
  Prefix(Vec<String>),
  /// Caller-provided predicate. Receives the verbatim header value.
  Custom(std::sync::Arc<dyn Fn(&str) -> bool + Send + Sync + 'static>),
}

impl ContentTypePolicy {
  pub(crate) fn matches(&self, ct: &str) -> bool {
    let ct = ct.split(';').next().unwrap_or(ct).trim();
    match self {
      Self::Default => {
        ct.starts_with("text/")
          || ct.contains("json")
          || ct.contains("javascript")
          || ct.contains("xml")
      }
      Self::Exact(list) => list.iter().any(|m| m.eq_ignore_ascii_case(ct)),
      Self::Prefix(list) => {
        let lc = ct.to_ascii_lowercase();
        list.iter().any(|m| lc.starts_with(&m.to_ascii_lowercase()))
      }
      Self::Custom(f) => f(ct),
    }
  }
}

/// Configuration settings for HTTP response compression.
#[derive(Clone)]
pub struct Config {
  /// List of enabled compression encodings in preference order.
  pub enabled: Vec<Encoding>,
  /// Minimum response size in bytes required for compression to be applied.
  pub min_size: usize,
  /// Gzip compression level (1-9, where 9 is maximum compression).
  pub gzip_level: u32,
  /// Brotli compression level (1-11, where 11 is maximum compression).
  pub brotli_level: u32,
  /// DEFLATE compression level (1-9, where 9 is maximum compression).
  pub deflate_level: u32,
  /// Zstandard compression level (1-22, where 22 is maximum compression).
  #[cfg(feature = "zstd")]
  pub zstd_level: i32,
  /// Whether to use streaming compression instead of buffering entire responses.
  pub stream: bool,
  /// Which response content types are eligible for compression.
  pub content_types: ContentTypePolicy,
  /// When true (default), responses that look like they carry authenticated
  /// secrets (Set-Cookie present, or the request had Authorization /
  /// Proxy-Authorization / Cookie) are *not* compressed. This is the
  /// canonical CRIME / BREACH mitigation. Disable explicitly with
  /// [`CompressionBuilder::protect_sensitive`](super::builder::CompressionBuilder::protect_sensitive) when you have other
  /// mitigations (e.g. per-response random padding or rotated CSRF tokens).
  pub protect_sensitive: bool,
}

impl Default for Config {
  /// Provides sensible default compression configuration.
  fn default() -> Self {
    Self {
      enabled: vec![Encoding::Gzip, Encoding::Brotli, Encoding::Deflate],
      min_size: 1024,
      gzip_level: 5,
      brotli_level: 5,
      deflate_level: 5,
      #[cfg(feature = "zstd")]
      zstd_level: 3,
      stream: false,
      content_types: ContentTypePolicy::default(),
      protect_sensitive: true,
    }
  }
}
