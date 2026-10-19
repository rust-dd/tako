#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
  Gzip,
  Brotli,
  /// The HTTP `deflate` coding, which is zlib-wrapped.
  Deflate,
  #[cfg(feature = "zstd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
  Zstd,
}

impl Encoding {
  /// The `Content-Encoding` token.
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
