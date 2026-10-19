use super::config::Config;
use super::config::ContentTypePolicy;
use super::encoding::Encoding;
use super::plugin::CompressionPlugin;

/// Defaults: Gzip, Brotli and DEFLATE at level 5 for buffered bodies of at least 1 KiB.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::CompressionBuilder;
///
/// // Basic setup with default settings
/// let basic = CompressionBuilder::new().build();
///
/// // Custom configuration
/// let custom = CompressionBuilder::new()
///     .enable_gzip(true)
///     .gzip_level(8)
///     .enable_brotli(true)
///     .brotli_level(6)
///     .enable_deflate(false)
///     .min_size(2048)
///     .enable_stream(true)
///     .build();
/// ```
pub struct CompressionBuilder(Config);

impl Default for CompressionBuilder {
  fn default() -> Self {
    Self::new()
  }
}

impl CompressionBuilder {
  pub fn new() -> Self {
    Self(Config::default())
  }

  fn toggle(mut self, encoding: Encoding, yes: bool) -> Self {
    if yes && !self.0.enabled.contains(&encoding) {
      self.0.enabled.push(encoding);
    }
    if !yes {
      self.0.enabled.retain(|e| *e != encoding);
    }
    self
  }

  pub fn enable_gzip(self, yes: bool) -> Self {
    self.toggle(Encoding::Gzip, yes)
  }

  pub fn enable_brotli(self, yes: bool) -> Self {
    self.toggle(Encoding::Brotli, yes)
  }

  pub fn enable_deflate(self, yes: bool) -> Self {
    self.toggle(Encoding::Deflate, yes)
  }

  #[cfg(feature = "zstd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
  pub fn enable_zstd(self, yes: bool) -> Self {
    self.toggle(Encoding::Zstd, yes)
  }

  /// Compresses chunk by chunk instead of buffering the whole body.
  pub fn enable_stream(mut self, stream: bool) -> Self {
    self.0.stream = stream;
    self
  }

  /// Smaller bodies are sent uncompressed; streams are checked only by `Content-Length`.
  pub fn min_size(mut self, bytes: usize) -> Self {
    self.0.min_size = bytes;
    self
  }

  /// Moves buffered compression to a blocking worker above this size.
  /// Defaults to 64 KiB. `usize::MAX` keeps compression on the request task.
  pub fn blocking_threshold(mut self, bytes: usize) -> Self {
    self.0.blocking_threshold = bytes;
    self
  }

  pub fn content_types(mut self, policy: ContentTypePolicy) -> Self {
    self.0.content_types = policy;
    self
  }

  /// Capped at 9.
  pub fn gzip_level(mut self, lvl: u32) -> Self {
    self.0.gzip_level = lvl.min(9);
    self
  }

  /// Capped at 11.
  pub fn brotli_level(mut self, lvl: u32) -> Self {
    self.0.brotli_level = lvl.min(11);
    self
  }

  /// Capped at 9.
  pub fn deflate_level(mut self, lvl: u32) -> Self {
    self.0.deflate_level = lvl.min(9);
    self
  }

  /// Clamped to 1–22.
  #[cfg(feature = "zstd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
  pub fn zstd_level(mut self, lvl: i32) -> Self {
    self.0.zstd_level = lvl.clamp(1, 22);
    self
  }

  /// CRIME/BREACH guard, on by default: responses that set cookies, or answer requests with
  /// credentials or cookies, stay uncompressed. Disable only with another mitigation.
  pub fn protect_sensitive(mut self, on: bool) -> Self {
    self.0.protect_sensitive = on;
    self
  }

  pub fn build(self) -> CompressionPlugin {
    CompressionPlugin { cfg: self.0 }
  }
}
