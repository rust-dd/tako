//! Fluent builder for assembling a [`CompressionPlugin`](super::plugin::CompressionPlugin).

use super::config::Config;
use super::config::ContentTypePolicy;
use super::encoding::Encoding;
use super::plugin::CompressionPlugin;

/// Builder for configuring HTTP response compression settings.
///
/// `CompressionBuilder` provides a fluent API for constructing compression plugin
/// configurations. It allows selective enabling/disabling of compression algorithms,
/// setting compression levels, and configuring behavior options like streaming and
/// minimum response size thresholds.
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
  /// Creates a new compression configuration builder with default settings.
  pub fn new() -> Self {
    Self(Config::default())
  }

  /// Enables or disables Gzip compression.
  pub fn enable_gzip(mut self, yes: bool) -> Self {
    if yes && !self.0.enabled.contains(&Encoding::Gzip) {
      self.0.enabled.push(Encoding::Gzip);
    }
    if !yes {
      self.0.enabled.retain(|e| *e != Encoding::Gzip);
    }
    self
  }

  /// Enables or disables Brotli compression.
  pub fn enable_brotli(mut self, yes: bool) -> Self {
    if yes && !self.0.enabled.contains(&Encoding::Brotli) {
      self.0.enabled.push(Encoding::Brotli);
    }
    if !yes {
      self.0.enabled.retain(|e| *e != Encoding::Brotli);
    }
    self
  }

  /// Enables or disables DEFLATE compression.
  pub fn enable_deflate(mut self, yes: bool) -> Self {
    if yes && !self.0.enabled.contains(&Encoding::Deflate) {
      self.0.enabled.push(Encoding::Deflate);
    }
    if !yes {
      self.0.enabled.retain(|e| *e != Encoding::Deflate);
    }
    self
  }

  /// Enables or disables Zstandard compression (requires zstd feature).
  #[cfg(feature = "zstd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
  pub fn enable_zstd(mut self, yes: bool) -> Self {
    if yes && !self.0.enabled.contains(&Encoding::Zstd) {
      self.0.enabled.push(Encoding::Zstd);
    }
    if !yes {
      self.0.enabled.retain(|e| *e != Encoding::Zstd);
    }
    self
  }

  /// Enables or disables streaming compression mode.
  pub fn enable_stream(mut self, stream: bool) -> Self {
    self.0.stream = stream;
    self
  }

  /// Sets the minimum response size threshold for compression.
  pub fn min_size(mut self, bytes: usize) -> Self {
    self.0.min_size = bytes;
    self
  }

  /// Replaces the content-type matching policy.
  pub fn content_types(mut self, policy: ContentTypePolicy) -> Self {
    self.0.content_types = policy;
    self
  }

  /// Sets the Gzip compression level (1-9).
  pub fn gzip_level(mut self, lvl: u32) -> Self {
    self.0.gzip_level = lvl.min(9);
    self
  }

  /// Sets the Brotli compression level (1-11).
  pub fn brotli_level(mut self, lvl: u32) -> Self {
    self.0.brotli_level = lvl.min(11);
    self
  }

  /// Sets the DEFLATE compression level (1-9).
  pub fn deflate_level(mut self, lvl: u32) -> Self {
    self.0.deflate_level = lvl.min(9);
    self
  }

  /// Sets the Zstandard compression level (1-22, requires zstd feature).
  #[cfg(feature = "zstd")]
  #[cfg_attr(docsrs, doc(cfg(feature = "zstd")))]
  pub fn zstd_level(mut self, lvl: i32) -> Self {
    self.0.zstd_level = lvl.clamp(1, 22);
    self
  }

  /// Toggle the CRIME/BREACH mitigation. Default is `true`: responses
  /// containing `Set-Cookie`, or whose request carried `Authorization`,
  /// `Proxy-Authorization`, or `Cookie`, are sent uncompressed. Setting this
  /// to `false` re-enables compression unconditionally — only do so if you
  /// have an alternative mitigation (per-response padding, rotated tokens).
  pub fn protect_sensitive(mut self, on: bool) -> Self {
    self.0.protect_sensitive = on;
    self
  }

  /// Builds the compression plugin with the configured settings.
  pub fn build(self) -> CompressionPlugin {
    CompressionPlugin { cfg: self.0 }
  }
}
