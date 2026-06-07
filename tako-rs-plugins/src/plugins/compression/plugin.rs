//! The compression plugin, its response wrapper, and the buffered/streaming middlewares.

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderValue;
use http::StatusCode;
use http::header::ACCEPT_ENCODING;
use http::header::CONTENT_ENCODING;
use http::header::CONTENT_LENGTH;
use http::header::CONTENT_TYPE;
use http::header::VARY;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::Next;
use tako_rs_core::plugins::TakoPlugin;
use tako_rs_core::responder::Responder;
use tako_rs_core::router::Router;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::brotli_stream::stream_brotli;
use super::config::Config;
use super::deflate_stream::stream_deflate;
use super::encoder::compress_brotli;
use super::encoder::compress_deflate;
use super::encoder::compress_gzip;
#[cfg(feature = "zstd")]
use super::encoder::compress_zstd;
use super::encoding::Encoding;
use super::gzip_stream::stream_gzip;
use super::negotiate::choose_encoding;
#[cfg(feature = "zstd")]
use super::zstd_stream::stream_zstd;

pub enum CompressionResponse<R>
where
  R: Responder,
{
  /// Plain, uncompressed response.
  Plain(R),
  /// Compressed or streaming response.
  Stream(R),
}

impl<R> Responder for CompressionResponse<R>
where
  R: Responder,
{
  fn into_response(self) -> Response {
    match self {
      CompressionResponse::Plain(r) => r.into_response(),
      CompressionResponse::Stream(r) => r.into_response(),
    }
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
#[doc(alias = "compression")]
#[doc(alias = "gzip")]
#[doc(alias = "brotli")]
#[doc(alias = "deflate")]
pub struct CompressionPlugin {
  pub(crate) cfg: Config,
}

impl Default for CompressionPlugin {
  /// Creates a compression plugin with default configuration settings.
  fn default() -> Self {
    Self {
      cfg: Config::default(),
    }
  }
}

#[async_trait]
impl TakoPlugin for CompressionPlugin {
  /// Returns the plugin name for identification and debugging.
  fn name(&self) -> &'static str {
    "CompressionPlugin"
  }

  /// Sets up the compression plugin by registering middleware with the router.
  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = self.cfg.clone();
    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      let stream = cfg.stream;
      async move {
        if stream {
          CompressionResponse::Stream(
            compress_stream_middleware(req, next, cfg)
              .await
              .into_response(),
          )
        } else {
          CompressionResponse::Plain(compress_middleware(req, next, cfg).await.into_response())
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
  let accepted = req
    .headers()
    .get(ACCEPT_ENCODING)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("")
    .to_ascii_lowercase();
  let request_is_authenticated = cfg.protect_sensitive && request_carries_credentials(&req);

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

  // CRIME/BREACH mitigation: compressing an authenticated response next to
  // attacker-controlled body content leaks the secret via the ciphertext
  // length. Skip compression entirely if either the request looked
  // authenticated or the response carries credentials.
  if cfg.protect_sensitive
    && (request_is_authenticated || resp.headers().contains_key(http::header::SET_COOKIE))
  {
    return resp.into_response();
  }

  // Skip compression for unsupported content types.
  if let Some(ct) = resp.headers().get(CONTENT_TYPE) {
    let ct = ct.to_str().unwrap_or("");
    if !cfg.content_types.matches(ct) {
      return resp.into_response();
    }
  }

  // The response is now compression-eligible. Always advertise that the
  // representation depends on `Accept-Encoding` so caches don't serve a
  // wrongly-encoded variant to a peer with different `Accept-Encoding`.
  ensure_vary_accept_encoding(resp.headers_mut());

  // Collect the response body and check its size.
  //
  // PPL-10: on body-collect failure the previous code overwrote the
  // handler's status with 502 and dropped the body. That obliterated any
  // intentional non-2xx the handler had produced — a 401, 404, or 503 from
  // the handler showed up to clients as 502, distorting downstream
  // metrics and observability. The collect-failure was specifically a
  // *compression-side* problem (the middleware could not buffer the body
  // for compression), not a downstream-gateway error.
  //
  // Better: keep the handler's original status, strip `Content-Encoding`
  // (we won't be compressing after all), warn so operators see the
  // failure, and return an empty body. The status truth survives; the
  // compression attempt is silently elided.
  let body_bytes = if let Ok(c) = resp.body_mut().collect().await {
    c.to_bytes()
  } else {
    tracing::warn!(
      "compression middleware: response body collect() failed; \
       returning original status with empty body (no compression)"
    );
    resp.headers_mut().remove(http::header::CONTENT_ENCODING);
    *resp.body_mut() = TakoBody::empty();
    return resp.into_response();
  };
  if body_bytes.len() < cfg.min_size {
    *resp.body_mut() = TakoBody::from(body_bytes);
    return resp.into_response();
  }

  // Compress the response body if a suitable encoding is chosen. If the
  // encoder fails (out-of-memory, malformed input, etc.) we MUST NOT set
  // `Content-Encoding` to the chosen scheme while serving the raw body —
  // the client would attempt to decode plain bytes as gzip/brotli and
  // fail. Track success explicitly and only advertise the encoding when
  // the compressed buffer was produced.
  if let Some(enc) = chosen {
    let compressed = match enc {
      Encoding::Gzip => compress_gzip(&body_bytes, cfg.gzip_level).ok(),
      Encoding::Brotli => compress_brotli(&body_bytes, cfg.brotli_level).ok(),
      Encoding::Deflate => compress_deflate(&body_bytes, cfg.deflate_level).ok(),
      #[cfg(feature = "zstd")]
      Encoding::Zstd => compress_zstd(&body_bytes, cfg.zstd_level).ok(),
    };
    if let Some(buf) = compressed {
      *resp.body_mut() = TakoBody::from(Bytes::from(buf));
      resp
        .headers_mut()
        .insert(CONTENT_ENCODING, HeaderValue::from_static(enc.as_str()));
      resp.headers_mut().remove(CONTENT_LENGTH);
    } else {
      tracing::warn!(
        encoding = enc.as_str(),
        "compression failed; serving identity"
      );
      *resp.body_mut() = TakoBody::from(body_bytes);
      resp.headers_mut().remove(CONTENT_ENCODING);
    }
  } else {
    *resp.body_mut() = TakoBody::from(body_bytes);
  }

  resp.into_response()
}

/// Middleware function for streaming response compression.
///
/// This middleware compresses response bodies on-the-fly as they stream to clients.
/// It's more memory-efficient than buffered compression but requires compatible
/// response body types that support streaming.
///
/// **Internal:** drop-shipped through `CompressionPlugin::setup` only. The
/// previous `pub` visibility was accidental — not re-exported from the
/// umbrella crate and not part of the documented API. Demoted to
/// `pub(crate)` so the public surface stays committed to the plugin entry
/// point. If you need this on its own use `CompressionPlugin` and let the
/// builder install it.
pub(crate) async fn compress_stream_middleware(
  req: Request,
  next: Next,
  cfg: Config,
) -> impl Responder {
  // Parse the `Accept-Encoding` header to determine supported encodings.
  let accepted = req
    .headers()
    .get(ACCEPT_ENCODING)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("")
    .to_ascii_lowercase();
  let request_is_authenticated = cfg.protect_sensitive && request_carries_credentials(&req);

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

  // CRIME/BREACH mitigation: see `compress_middleware`.
  if cfg.protect_sensitive
    && (request_is_authenticated || resp.headers().contains_key(http::header::SET_COOKIE))
  {
    return resp.into_response();
  }

  // Skip compression for unsupported content types.
  if let Some(ct) = resp.headers().get(CONTENT_TYPE) {
    let ct = ct.to_str().unwrap_or("");
    if !cfg.content_types.matches(ct) {
      return resp.into_response();
    }
  }

  // The response is compression-eligible: advertise Vary regardless of whether we
  // actually apply an encoding, so caches key on `Accept-Encoding`.
  ensure_vary_accept_encoding(resp.headers_mut());

  // Estimate size from `Content-Length`.
  if let Some(len) = resp
    .headers()
    .get(CONTENT_LENGTH)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.parse::<usize>().ok())
    && len < cfg.min_size
  {
    return resp.into_response();
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
    resp
      .headers_mut()
      .insert(CONTENT_ENCODING, HeaderValue::from_static(enc.as_str()));
    resp.headers_mut().remove(CONTENT_LENGTH);
  }

  resp.into_response()
}

/// Returns true if the request carries credentials that would make its
/// response a CRIME/BREACH target. The check is intentionally broad: any
/// auth header or cookie is treated as authenticated.
fn request_carries_credentials(req: &Request) -> bool {
  req.headers().contains_key(http::header::AUTHORIZATION)
    || req
      .headers()
      .contains_key(http::header::PROXY_AUTHORIZATION)
    || req.headers().contains_key(http::header::COOKIE)
}

/// Appends `Accept-Encoding` to the `Vary` header without duplicating it.
///
/// `Vary: Accept-Encoding` is required on every compression-eligible response
/// so shared caches don't serve a wrongly-encoded representation to a different
/// client.
fn ensure_vary_accept_encoding(headers: &mut http::HeaderMap) {
  let already_present = headers.get_all(VARY).iter().any(|v| {
    v.to_str().is_ok_and(|s| {
      s.split(',')
        .any(|tok| tok.trim().eq_ignore_ascii_case("Accept-Encoding"))
    })
  });
  if !already_present {
    headers.append(VARY, HeaderValue::from_static("Accept-Encoding"));
  }
}
