//! HTTP response compression plugin supporting multiple algorithms and streaming.
//!
//! This module provides comprehensive HTTP response compression functionality for Tako
//! applications. It supports multiple compression algorithms including Gzip, Brotli, DEFLATE,
//! and optionally Zstandard, with configurable compression levels and streaming capabilities.
//! The plugin automatically negotiates compression based on client Accept-Encoding headers
//! and applies compression selectively based on content type, response size, and status code.
//!
//! # Examples
//!
//! ```rust
//! use tako::plugins::compression::CompressionBuilder;
//! use tako::plugins::TakoPlugin;
//! use tako::router::Router;
//!
//! // Basic compression setup
//! let compression = CompressionBuilder::new()
//!     .enable_gzip(true)
//!     .enable_brotli(true)
//!     .min_size(1024)
//!     .build();
//!
//! let mut router = Router::new();
//! router.plugin(compression);
//!
//! // Advanced compression configuration
//! let advanced = CompressionBuilder::new()
//!     .enable_gzip(true)
//!     .gzip_level(6)
//!     .enable_brotli(true)
//!     .brotli_level(4)
//!     .enable_stream(true)
//!     .min_size(512)
//!     .build();
//! ```

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use flate2::{
    Compression as GzLevel,
    write::{DeflateEncoder, GzEncoder},
};
use http::{
    HeaderValue, StatusCode,
    header::{ACCEPT_ENCODING, CONTENT_ENCODING, CONTENT_LENGTH, CONTENT_TYPE, VARY},
};
use http_body_util::BodyExt;
use std::io::{Read, Write};

pub mod brotli_stream;
pub mod deflate_stream;
pub mod gzip_stream;
pub mod zstd_stream;

#[cfg(feature = "zstd")]
use zstd::stream::encode_all as zstd_encode;

#[cfg(feature = "zstd")]
use crate::plugins::compression::zstd_stream::stream_zstd;
use crate::{
    body::TakoBody,
    middleware::Next,
    plugins::{
        TakoPlugin,
        compression::{
            brotli_stream::stream_brotli, deflate_stream::stream_deflate, gzip_stream::stream_gzip,
        },
    },
    responder::{CompressionResponse, Responder},
    router::Router,
    types::Request,
};

/// Supported HTTP compression encoding algorithms.
///
/// This enum represents the different compression algorithms supported by the compression
/// plugin. Each encoding corresponds to a standard HTTP Content-Encoding value and
/// provides different trade-offs between compression ratio, speed, and compatibility.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::Encoding;
///
/// let gzip = Encoding::Gzip;
/// assert_eq!(gzip.as_str(), "gzip");
///
/// let brotli = Encoding::Brotli;
/// assert_eq!(brotli.as_str(), "br");
/// ```
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
    Zstd,
}

impl Encoding {
    /// Returns the HTTP Content-Encoding header value for this compression algorithm.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::Encoding;
    ///
    /// assert_eq!(Encoding::Gzip.as_str(), "gzip");
    /// assert_eq!(Encoding::Brotli.as_str(), "br");
    /// assert_eq!(Encoding::Deflate.as_str(), "deflate");
    /// ```
    fn as_str(&self) -> &'static str {
        match self {
            Encoding::Gzip => "gzip",
            Encoding::Brotli => "br",
            Encoding::Deflate => "deflate",
            #[cfg(feature = "zstd")]
            Encoding::Zstd => "zstd",
        }
    }
}

/// Configuration settings for HTTP response compression.
///
/// This struct defines all configurable aspects of the compression plugin including
/// enabled algorithms, compression levels, minimum response sizes, and streaming options.
/// It provides sensible defaults while allowing fine-tuned control over compression behavior.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::{Config, Encoding};
///
/// let config = Config {
///     enabled: vec![Encoding::Gzip, Encoding::Brotli],
///     min_size: 2048,
///     gzip_level: 6,
///     brotli_level: 4,
///     deflate_level: 6,
///     stream: true,
///     ..Default::default()
/// };
/// ```
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
}

impl Default for Config {
    /// Provides sensible default compression configuration.
    ///
    /// Default settings enable Gzip, Brotli, and DEFLATE with balanced compression levels,
    /// 1KB minimum size threshold, and buffered (non-streaming) compression.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::{Config, Encoding};
    ///
    /// let config = Config::default();
    /// assert_eq!(config.min_size, 1024);
    /// assert_eq!(config.gzip_level, 5);
    /// assert!(!config.stream);
    /// ```
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
        }
    }
}

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

impl CompressionBuilder {
    /// Creates a new compression configuration builder with default settings.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let builder = CompressionBuilder::new();
    /// let compression = builder.build();
    /// ```
    pub fn new() -> Self {
        Self(Config::default())
    }

    /// Enables or disables Gzip compression.
    ///
    /// When enabled, Gzip compression will be available for client negotiation.
    /// Gzip offers wide browser compatibility and good compression ratios.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .enable_gzip(true)
    ///     .build();
    /// ```
    pub fn enable_gzip(mut self, yes: bool) -> Self {
        if yes && !self.0.enabled.contains(&Encoding::Gzip) {
            self.0.enabled.push(Encoding::Gzip)
        }
        if !yes {
            self.0.enabled.retain(|e| *e != Encoding::Gzip)
        }
        self
    }

    /// Enables or disables Brotli compression.
    ///
    /// When enabled, Brotli compression will be available for client negotiation.
    /// Brotli provides excellent compression ratios and is supported by modern browsers.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .enable_brotli(true)
    ///     .build();
    /// ```
    pub fn enable_brotli(mut self, yes: bool) -> Self {
        if yes && !self.0.enabled.contains(&Encoding::Brotli) {
            self.0.enabled.push(Encoding::Brotli)
        }
        if !yes {
            self.0.enabled.retain(|e| *e != Encoding::Brotli)
        }
        self
    }

    /// Enables or disables DEFLATE compression.
    ///
    /// When enabled, DEFLATE compression will be available for client negotiation.
    /// DEFLATE offers fast compression with good compatibility across HTTP clients.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .enable_deflate(true)
    ///     .build();
    /// ```
    pub fn enable_deflate(mut self, yes: bool) -> Self {
        if yes && !self.0.enabled.contains(&Encoding::Deflate) {
            self.0.enabled.push(Encoding::Deflate)
        }
        if !yes {
            self.0.enabled.retain(|e| *e != Encoding::Deflate)
        }
        self
    }

    /// Enables or disables Zstandard compression (requires zstd feature).
    ///
    /// When enabled, Zstandard compression will be available for client negotiation.
    /// Zstd provides excellent compression ratios with fast compression/decompression.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "zstd")]
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// # #[cfg(feature = "zstd")]
    /// let compression = CompressionBuilder::new()
    ///     .enable_zstd(true)
    ///     .build();
    /// ```
    #[cfg(feature = "zstd")]
    pub fn enable_zstd(mut self, yes: bool) -> Self {
        if yes && !self.0.enabled.contains(&Encoding::Zstd) {
            self.0.enabled.push(Encoding::Zstd)
        }
        if !yes {
            self.0.enabled.retain(|e| *e != Encoding::Zstd)
        }
        self
    }

    /// Enables or disables streaming compression mode.
    ///
    /// When streaming is enabled, responses are compressed on-the-fly without
    /// buffering the entire response in memory. This is more memory-efficient
    /// for large responses but may have slightly higher CPU overhead.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let streaming = CompressionBuilder::new()
    ///     .enable_stream(true)
    ///     .build();
    /// ```
    pub fn enable_stream(mut self, stream: bool) -> Self {
        self.0.stream = stream;
        self
    }

    /// Sets the minimum response size threshold for compression.
    ///
    /// Responses smaller than this size will not be compressed, as the overhead
    /// of compression may exceed the bandwidth savings for small responses.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .min_size(2048) // Only compress responses >= 2KB
    ///     .build();
    /// ```
    pub fn min_size(mut self, bytes: usize) -> Self {
        self.0.min_size = bytes;
        self
    }

    /// Sets the Gzip compression level (1-9).
    ///
    /// Higher levels provide better compression at the cost of increased CPU usage.
    /// Level 6 is typically a good balance for web content.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .gzip_level(8) // High compression
    ///     .build();
    /// ```
    pub fn gzip_level(mut self, lvl: u32) -> Self {
        self.0.gzip_level = lvl.min(9);
        self
    }

    /// Sets the Brotli compression level (1-11).
    ///
    /// Higher levels provide better compression at the cost of increased CPU usage.
    /// Level 4-6 is typically recommended for web content.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .brotli_level(6) // Balanced compression
    ///     .build();
    /// ```
    pub fn brotli_level(mut self, lvl: u32) -> Self {
        self.0.brotli_level = lvl.min(11);
        self
    }

    /// Sets the DEFLATE compression level (1-9).
    ///
    /// Higher levels provide better compression at the cost of increased CPU usage.
    /// Level 6 is typically a good balance for web content.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .deflate_level(7) // High compression
    ///     .build();
    /// ```
    pub fn deflate_level(mut self, lvl: u32) -> Self {
        self.0.deflate_level = lvl.min(9);
        self
    }

    /// Sets the Zstandard compression level (1-22, requires zstd feature).
    ///
    /// Higher levels provide better compression at the cost of increased CPU usage.
    /// Level 3-6 is typically recommended for web content.
    ///
    /// # Examples
    ///
    /// ```rust
    /// # #[cfg(feature = "zstd")]
    /// use tako::plugins::compression::CompressionBuilder;
    ///
    /// # #[cfg(feature = "zstd")]
    /// let compression = CompressionBuilder::new()
    ///     .zstd_level(6) // Balanced compression
    ///     .build();
    /// ```
    #[cfg(feature = "zstd")]
    pub fn zstd_level(mut self, lvl: i32) -> Self {
        self.0.zstd_level = lvl.clamp(1, 22);
        self
    }

    /// Builds the compression plugin with the configured settings.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionBuilder;
    /// use tako::plugins::TakoPlugin;
    /// use tako::router::Router;
    ///
    /// let compression = CompressionBuilder::new()
    ///     .enable_gzip(true)
    ///     .gzip_level(6)
    ///     .min_size(1024)
    ///     .build();
    ///
    /// let mut router = Router::new();
    /// router.plugin(compression);
    /// ```
    pub fn build(self) -> CompressionPlugin {
        CompressionPlugin { cfg: self.0 }
    }
}

/// HTTP response compression plugin for Tako applications.
///
/// `CompressionPlugin` provides automatic response compression based on client
/// Accept-Encoding headers and configurable compression algorithms. It supports
/// multiple compression formats, streaming compression, and intelligent content
/// type detection to optimize bandwidth usage and response times.
///
/// # Examples
///
/// ```rust
/// use tako::plugins::compression::{CompressionPlugin, CompressionBuilder};
/// use tako::plugins::TakoPlugin;
/// use tako::router::Router;
///
/// // Use default settings
/// let compression = CompressionPlugin::default();
/// let mut router = Router::new();
/// router.plugin(compression);
///
/// // Custom configuration
/// let custom = CompressionBuilder::new()
///     .enable_gzip(true)
///     .enable_brotli(true)
///     .min_size(2048)
///     .build();
/// router.plugin(custom);
/// ```
#[derive(Clone)]
pub struct CompressionPlugin {
    cfg: Config,
}

impl Default for CompressionPlugin {
    /// Creates a compression plugin with default configuration settings.
    ///
    /// Default settings enable Gzip, Brotli, and DEFLATE compression with balanced
    /// compression levels and a 1KB minimum size threshold.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionPlugin;
    /// use tako::plugins::TakoPlugin;
    /// use tako::router::Router;
    ///
    /// let compression = CompressionPlugin::default();
    /// let mut router = Router::new();
    /// router.plugin(compression);
    /// ```
    fn default() -> Self {
        Self {
            cfg: Config::default(),
        }
    }
}

#[async_trait]
impl TakoPlugin for CompressionPlugin {
    /// Returns the plugin name for identification and debugging.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionPlugin;
    /// use tako::plugins::TakoPlugin;
    ///
    /// let plugin = CompressionPlugin::default();
    /// assert_eq!(plugin.name(), "CompressionPlugin");
    /// ```
    fn name(&self) -> &'static str {
        "CompressionPlugin"
    }

    /// Sets up the compression plugin by registering middleware with the router.
    ///
    /// This method installs the compression middleware that will automatically
    /// compress responses based on the plugin configuration and client capabilities.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::plugins::compression::CompressionPlugin;
    /// use tako::plugins::TakoPlugin;
    /// use tako::router::Router;
    ///
    /// let plugin = CompressionPlugin::default();
    /// let router = Router::new();
    /// plugin.setup(&router).unwrap();
    /// ```
    fn setup(&self, router: &Router) -> Result<()> {
        let cfg = self.cfg.clone();
        router.middleware(move |req, next| {
            let cfg = cfg.clone();
            let stream = cfg.stream.clone();
            async move {
                if stream == false {
                    return CompressionResponse::Plain(
                        compress_middleware(req, next, cfg).await.into_response(),
                    );
                } else {
                    return CompressionResponse::Stream(
                        compress_stream_middleware(req, next, cfg)
                            .await
                            .into_response(),
                    );
                }
            }
        });
        Ok(())
    }
}

/// Middleware function for buffered response compression.
///
/// This middleware compresses entire response bodies in memory before sending them
/// to clients. It's more memory-intensive than streaming compression but may have
/// better compression ratios for smaller responses.
async fn compress_middleware(req: Request, next: Next, cfg: Config) -> impl Responder {
    // Parse the `Accept-Encoding` header to determine supported encodings.
    let accepted = req
        .headers()
        .get(ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Process the request and get the response.
    let mut resp = next.run(req).await;
    let chosen = choose_encoding(&accepted, &cfg.enabled);

    // Skip compression for non-successful responses or if already encoded.
    let status = resp.status();
    if !(status.is_success() || status == StatusCode::NOT_MODIFIED) {
        return resp.into_response();
    }

    if resp.headers().contains_key(CONTENT_ENCODING) {
        return resp.into_response();
    }

    // Skip compression for unsupported content types.
    if let Some(ct) = resp.headers().get(CONTENT_TYPE) {
        let ct = ct.to_str().unwrap_or("");
        if !(ct.starts_with("text/")
            || ct.contains("json")
            || ct.contains("javascript")
            || ct.contains("xml"))
        {
            return resp.into_response();
        }
    }

    // Collect the response body and check its size.
    let body_bytes = resp.body_mut().collect().await.unwrap().to_bytes();
    if body_bytes.len() < cfg.min_size {
        *resp.body_mut() = TakoBody::from(Bytes::from(body_bytes));
        return resp.into_response();
    }

    // Compress the response body if a suitable encoding is chosen.
    if let Some(enc) = chosen {
        let compressed =
            match enc {
                Encoding::Gzip => compress_gzip(&body_bytes, cfg.gzip_level)
                    .unwrap_or_else(|_| body_bytes.to_vec()),
                Encoding::Brotli => compress_brotli(&body_bytes, cfg.brotli_level)
                    .unwrap_or_else(|_| body_bytes.to_vec()),
                Encoding::Deflate => compress_deflate(&body_bytes, cfg.deflate_level)
                    .unwrap_or_else(|_| body_bytes.to_vec()),
                #[cfg(feature = "zstd")]
                Encoding::Zstd => compress_zstd(&body_bytes, cfg.zstd_level)
                    .unwrap_or_else(|_| body_bytes.to_vec()),
            };
        *resp.body_mut() = TakoBody::from(Bytes::from(compressed));
        resp.headers_mut()
            .insert(CONTENT_ENCODING, HeaderValue::from_static(enc.as_str()));
        resp.headers_mut().remove(CONTENT_LENGTH);
        resp.headers_mut()
            .insert(VARY, HeaderValue::from_static("Accept-Encoding"));
    } else {
        *resp.body_mut() = TakoBody::from(Bytes::from(body_bytes));
    }

    resp.into_response()
}

/// Middleware function for streaming response compression.
///
/// This middleware compresses response bodies on-the-fly as they stream to clients.
/// It's more memory-efficient than buffered compression but requires compatible
/// response body types that support streaming.
pub async fn compress_stream_middleware(req: Request, next: Next, cfg: Config) -> impl Responder {
    // Parse the `Accept-Encoding` header to determine supported encodings.
    let accepted = req
        .headers()
        .get(ACCEPT_ENCODING)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_ascii_lowercase();

    // Process the request and get the response.
    let mut resp = next.run(req).await;
    let chosen = choose_encoding(&accepted, &cfg.enabled);

    // Skip compression for non-successful responses or if already encoded.
    let status = resp.status();
    if !(status.is_success() || status == StatusCode::NOT_MODIFIED) {
        return resp.into_response();
    }

    if resp.headers().contains_key(CONTENT_ENCODING) {
        return resp.into_response();
    }

    // Skip compression for unsupported content types.
    if let Some(ct) = resp.headers().get(CONTENT_TYPE) {
        let ct = ct.to_str().unwrap_or("");
        if !(ct.starts_with("text/")
            || ct.contains("json")
            || ct.contains("javascript")
            || ct.contains("xml"))
        {
            return resp.into_response();
        }
    }

    // Estimate size from `Content-Length`.
    if let Some(len) = resp
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<usize>().ok())
    {
        if len < cfg.min_size {
            return resp.into_response();
        }
    }

    if let Some(enc) = chosen {
        let body = std::mem::replace(resp.body_mut(), TakoBody::empty());
        let new_body = match enc {
            Encoding::Gzip => stream_gzip(body, cfg.gzip_level),
            Encoding::Brotli => stream_brotli(body, cfg.brotli_level),
            Encoding::Deflate => stream_deflate(body, cfg.deflate_level),
            #[cfg(feature = "zstd")]
            Encoding::Zstd => stream_zstd(body, cfg.zstd_level),
        };
        *resp.body_mut() = new_body;
        resp.headers_mut()
            .insert(CONTENT_ENCODING, HeaderValue::from_static(enc.as_str()));
        resp.headers_mut().remove(CONTENT_LENGTH);
        resp.headers_mut()
            .insert(VARY, HeaderValue::from_static("Accept-Encoding"));
    }

    resp.into_response()
}

/// Selects the best compression encoding based on client preferences and server capabilities.
///
/// This function parses the Accept-Encoding header and chooses the most preferred
/// compression algorithm that is both supported by the client and enabled on the server.
/// The selection prioritizes compression quality while respecting client preferences.
fn choose_encoding(header: &str, enabled: &[Encoding]) -> Option<Encoding> {
    let header = header.to_ascii_lowercase();
    let test = |e: Encoding| header.contains(e.as_str()) && enabled.contains(&e);
    if test(Encoding::Brotli) {
        Some(Encoding::Brotli)
    } else if test(Encoding::Gzip) {
        Some(Encoding::Gzip)
    } else if test(Encoding::Deflate) {
        Some(Encoding::Deflate)
    } else {
        #[cfg(feature = "zstd")]
        {
            if test(Encoding::Zstd) {
                return Some(Encoding::Zstd);
            }
        }
        None
    }
}

/// Compresses data using Gzip algorithm.
///
/// This function performs complete Gzip compression of the input data and returns
/// the compressed result. It's used for buffered compression mode.
fn compress_gzip(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), GzLevel::new(lvl));
    enc.write_all(data)?;
    enc.finish()
}

/// Compresses data using Brotli algorithm.
///
/// This function performs complete Brotli compression of the input data and returns
/// the compressed result. It's used for buffered compression mode.
fn compress_brotli(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::CompressorReader::new(data, 4096, lvl, 22)
        .read_to_end(&mut out)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to compress data"))?;
    Ok(out)
}

/// Compresses data using DEFLATE algorithm.
///
/// This function performs complete DEFLATE compression of the input data and returns
/// the compressed result. It's used for buffered compression mode.
fn compress_deflate(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::new(lvl));
    enc.write_all(data)?;
    enc.finish()
}

/// Compresses data using Zstandard algorithm (requires zstd feature).
///
/// This function performs complete Zstandard compression of the input data and returns
/// the compressed result. It's used for buffered compression mode.
#[cfg(feature = "zstd")]
fn compress_zstd(data: &[u8], lvl: i32) -> std::io::Result<Vec<u8>> {
    zstd_encode(data, lvl)
}
