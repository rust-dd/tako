use super::encoding::Encoding;

#[derive(Clone, Default)]
pub enum ContentTypePolicy {
  /// Default heuristic: text/*, anything containing `json`, `javascript`, `xml`.
  #[default]
  Default,
  /// Exact MIME types (case-insensitive). E.g. `["application/json", "text/html"]`.
  Exact(Vec<String>),
  /// MIME prefixes (case-insensitive). E.g. `["text/", "application/x-json-"]`.
  Prefix(Vec<String>),
  /// Receives the MIME type without parameters.
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

#[derive(Clone)]
pub struct Config {
  /// Order is ignored: negotiation prefers Brotli, Gzip, DEFLATE, then Zstandard.
  pub enabled: Vec<Encoding>,
  /// Bytes; smaller bodies are sent uncompressed.
  pub min_size: usize,
  /// Buffered bodies at least this large are compressed on a blocking worker.
  pub blocking_threshold: usize,
  /// Gzip compression level (1-9, where 9 is maximum compression).
  pub gzip_level: u32,
  /// Brotli compression level (1-11, where 11 is maximum compression).
  pub brotli_level: u32,
  /// DEFLATE compression level (1-9, where 9 is maximum compression).
  pub deflate_level: u32,
  /// Zstandard compression level (1-22, where 22 is maximum compression).
  #[cfg(feature = "zstd")]
  pub zstd_level: i32,
  /// Compress chunk by chunk instead of buffering the whole body.
  pub stream: bool,
  pub content_types: ContentTypePolicy,
  /// Skip compression when the response sets cookies or the request carried credentials.
  pub protect_sensitive: bool,
}

impl Default for Config {
  fn default() -> Self {
    Self {
      enabled: vec![Encoding::Gzip, Encoding::Brotli, Encoding::Deflate],
      min_size: 1024,
      blocking_threshold: 64 * 1024,
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
