/// The `CompressionPlugin` provides response compression functionality for the Tako framework.
/// It supports Gzip, Brotli, and optionally Zstd compression, allowing you to optimize response sizes
/// for faster client-side loading and reduced bandwidth usage. The plugin is highly configurable,
/// enabling you to set compression levels, minimum response sizes, and supported encodings.
///
/// # Example
/// ```rust
/// use tako::plugins::compression::CompressionBuilder;
///
/// let compression = CompressionBuilder::new()
///     .enable_gzip(true)
///     .enable_brotli(true)
///     .min_size(1024)
///     .gzip_level(6)
///     .brotli_level(4)
///     .build();
///
/// router.plugin(compression);
/// ```
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

/// Supported compression encodings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Encoding {
    Gzip,
    Brotli,
    Deflate,
    #[cfg(feature = "zstd")]
    Zstd,
}

impl Encoding {
    /// Returns the string representation of the encoding.
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

/// Configuration for the compression plugin.
#[derive(Clone)]
pub struct Config {
    /// List of enabled compression encodings.
    pub enabled: Vec<Encoding>,
    /// Minimum size (in bytes) for a response to be compressed.
    pub min_size: usize,
    /// Compression level for Gzip.
    pub gzip_level: u32,
    /// Compression level for Brotli.
    pub brotli_level: u32,
    /// Compression level for Deflate.
    pub deflate_level: u32,
    /// Compression level for Zstd (if enabled).
    #[cfg(feature = "zstd")]
    pub zstd_level: i32,
    /// Whether to use streaming compression.
    pub stream: bool,
}

impl Default for Config {
    /// Provides default configuration values.
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

/// Builder for configuring and creating a `CompressionPlugin`.
pub struct CompressionBuilder(Config);

impl CompressionBuilder {
    /// Creates a new builder with default configuration.
    pub fn new() -> Self {
        Self(Config::default())
    }

    /// Enables or disables Gzip compression.
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
    pub fn enable_brotli(mut self, yes: bool) -> Self {
        if yes && !self.0.enabled.contains(&Encoding::Brotli) {
            self.0.enabled.push(Encoding::Brotli)
        }
        if !yes {
            self.0.enabled.retain(|e| *e != Encoding::Brotli)
        }
        self
    }

    /// Enables or disables Deflate compression.
    pub fn enable_deflate(mut self, yes: bool) -> Self {
        if yes && !self.0.enabled.contains(&Encoding::Deflate) {
            self.0.enabled.push(Encoding::Deflate)
        }
        if !yes {
            self.0.enabled.retain(|e| *e != Encoding::Deflate)
        }
        self
    }

    /// Enables or disables Zstd compression (if supported).
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

    /// Sets whether to use streaming compression.
    pub fn enable_stream(mut self, stream: bool) -> Self {
        self.0.stream = stream;
        self
    }

    /// Sets the minimum response size (in bytes) for compression.
    pub fn min_size(mut self, bytes: usize) -> Self {
        self.0.min_size = bytes;
        self
    }

    /// Sets the compression level for Gzip.
    pub fn gzip_level(mut self, lvl: u32) -> Self {
        self.0.gzip_level = lvl.min(9);
        self
    }

    /// Sets the compression level for Brotli.
    pub fn brotli_level(mut self, lvl: u32) -> Self {
        self.0.brotli_level = lvl.min(11);
        self
    }

    /// Sets the compression level for Deflate.
    pub fn deflate_level(mut self, lvl: u32) -> Self {
        self.0.deflate_level = lvl.min(9);
        self
    }

    /// Sets the compression level for Zstd (if supported).
    #[cfg(feature = "zstd")]
    pub fn zstd_level(mut self, lvl: i32) -> Self {
        self.0.zstd_level = lvl.clamp(1, 22);
        self
    }

    /// Builds and returns the `CompressionPlugin` with the configured settings.
    pub fn build(self) -> CompressionPlugin {
        CompressionPlugin { cfg: self.0 }
    }
}

/// Plugin for handling response compression.
#[derive(Clone)]
pub struct CompressionPlugin {
    cfg: Config,
}

impl Default for CompressionPlugin {
    /// Creates a `CompressionPlugin` with default configuration.
    fn default() -> Self {
        Self {
            cfg: Config::default(),
        }
    }
}

#[async_trait]
impl TakoPlugin for CompressionPlugin {
    /// Returns the name of the plugin.
    fn name(&self) -> &'static str {
        "CompressionPlugin"
    }

    /// Sets up the plugin by adding the compression middleware to the router.
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

/// Middleware function for compressing responses.
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

/// Middleware function for compressing responses with streaming.
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

/// Chooses the best encoding based on the `Accept-Encoding` header and enabled encodings.
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

/// Compresses data using Gzip.
fn compress_gzip(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
    let mut enc = GzEncoder::new(Vec::new(), GzLevel::new(lvl));
    enc.write_all(data)?;
    enc.finish()
}

/// Compresses data using Brotli.
fn compress_brotli(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
    let mut out = Vec::new();
    brotli::CompressorReader::new(data, 4096, lvl, 22)
        .read_to_end(&mut out)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::Other, "Failed to compress data"))?;
    Ok(out)
}

/// Compresses data using Deflate.
fn compress_deflate(data: &[u8], lvl: u32) -> std::io::Result<Vec<u8>> {
    let mut enc = DeflateEncoder::new(Vec::new(), flate2::Compression::new(lvl));
    enc.write_all(data)?;
    enc.finish()
}

/// Compresses data using Zstd (if supported).
#[cfg(feature = "zstd")]
fn compress_zstd(data: &[u8], lvl: i32) -> std::io::Result<Vec<u8>> {
    zstd_encode(data, lvl)
}
