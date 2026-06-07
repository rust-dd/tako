//! Supported HTTP compression encodings and their header identities.

/// Supported HTTP compression encoding algorithms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
  /// Gzip compression (RFC 1952) - widely supported, good compression ratio.
  Gzip,
  /// Brotli compression (RFC 7932) - excellent compression ratio, modern browsers.
  Brotli,
  /// DEFLATE compression (RFC 1951) - fast compression, good compatibility.
  Deflate,
  /// Zstandard compression - high performance, excellent ratio (requires zstd feature).
  #[cfg(feature = "zstd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
  Zstd,
}

impl Encoding {
  /// Returns the HTTP Content-Encoding header value for this compression algorithm.
  pub(crate) fn as_str(&self) -> &'static str {
    match self {
      Encoding::Gzip => "gzip",
      Encoding::Brotli => "br",
      Encoding::Deflate => "deflate",
      #[cfg(feature = "zstd")]
      Encoding::Zstd => "zstd",
    }
  }
}
