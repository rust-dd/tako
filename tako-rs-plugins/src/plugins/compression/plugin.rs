use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use bytes::Bytes;
use http::HeaderValue;
use http::StatusCode;
use http::header::ACCEPT_ENCODING;
use http::header::ACCEPT_RANGES;
use http::header::CONTENT_ENCODING;
use http::header::CONTENT_LENGTH;
use http::header::CONTENT_RANGE;
use http::header::CONTENT_TYPE;
use http::header::VARY;
use http_body::Body;
use http_body_util::BodyExt;
use tako_rs_core::body::TakoBody;
use tako_rs_core::middleware::Next;
use tako_rs_core::plugins::TakoPlugin;
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
#[derive(Clone, Default)]
#[doc(alias = "compression")]
#[doc(alias = "gzip")]
#[doc(alias = "brotli")]
#[doc(alias = "deflate")]
pub struct CompressionPlugin {
  pub(crate) cfg: Config,
}

#[async_trait]
impl TakoPlugin for CompressionPlugin {
  fn name(&self) -> &'static str {
    "CompressionPlugin"
  }

  fn setup(&self, router: &Router) -> Result<()> {
    let cfg = Arc::new(self.cfg.clone());
    router.middleware(move |req, next| {
      let cfg = cfg.clone();
      async move {
        if cfg.stream {
          compress_stream_middleware(req, next, cfg).await
        } else {
          compress_middleware(req, next, cfg).await
        }
      }
    });
    Ok(())
  }
}

async fn compress_middleware(req: Request, next: Next, cfg: Arc<Config>) -> Response {
  let chosen = negotiate(&req, &cfg);
  let request_is_authenticated = cfg.protect_sensitive && request_carries_credentials(&req);

  let mut resp = next.run(req).await;
  if !eligible(&resp, &cfg, request_is_authenticated) || is_event_stream(&resp) {
    return resp;
  }

  // Open-ended bodies must keep yielding frames without waiting for stream completion.
  if resp.body().size_hint().exact().is_none() {
    return resp;
  }

  // Eligible from here on, so caches must key on `Accept-Encoding` even when none applies.
  ensure_vary_accept_encoding(resp.headers_mut());

  let Some(enc) = chosen else {
    return resp;
  };

  let body_bytes = if let Ok(c) = resp.body_mut().collect().await {
    c.to_bytes()
  } else {
    tracing::warn!(
      "compression middleware: response body collect() failed; \
       returning original status with empty body (no compression)"
    );
    resp.headers_mut().remove(http::header::CONTENT_ENCODING);
    resp.headers_mut().remove(CONTENT_LENGTH);
    *resp.body_mut() = TakoBody::empty();
    return resp;
  };
  if body_bytes.len() < cfg.min_size {
    *resp.body_mut() = TakoBody::from(body_bytes);
    return resp;
  }

  let compressed = if body_bytes.len() >= cfg.blocking_threshold {
    let input = body_bytes.clone();
    #[cfg(not(feature = "compio"))]
    let task = tokio::task::spawn_blocking(move || compress_buffer(&input, enc, &cfg));
    #[cfg(feature = "compio")]
    let task = compio::runtime::spawn_blocking(move || compress_buffer(&input, enc, &cfg));
    task.await.ok().flatten()
  } else {
    compress_buffer(&body_bytes, enc, &cfg)
  };
  if let Some(buf) = compressed {
    *resp.body_mut() = TakoBody::from(Bytes::from(buf));
    resp
      .headers_mut()
      .insert(CONTENT_ENCODING, HeaderValue::from_static(enc.as_str()));
    resp.headers_mut().remove(CONTENT_LENGTH);
    resp.headers_mut().remove(ACCEPT_RANGES);
  } else {
    tracing::warn!(
      encoding = enc.as_str(),
      "compression failed; serving identity"
    );
    *resp.body_mut() = TakoBody::from(body_bytes);
    resp.headers_mut().remove(CONTENT_ENCODING);
  }

  resp
}

fn compress_buffer(body: &Bytes, encoding: Encoding, cfg: &Config) -> Option<Vec<u8>> {
  match encoding {
    Encoding::Gzip => compress_gzip(body, cfg.gzip_level).ok(),
    Encoding::Brotli => compress_brotli(body, cfg.brotli_level).ok(),
    Encoding::Deflate => compress_deflate(body, cfg.deflate_level).ok(),
    #[cfg(feature = "zstd")]
    Encoding::Zstd => compress_zstd(body, cfg.zstd_level).ok(),
  }
}

async fn compress_stream_middleware(req: Request, next: Next, cfg: Arc<Config>) -> Response {
  let chosen = negotiate(&req, &cfg);
  let request_is_authenticated = cfg.protect_sensitive && request_carries_credentials(&req);

  let mut resp = next.run(req).await;
  if !eligible(&resp, &cfg, request_is_authenticated) {
    return resp;
  }

  ensure_vary_accept_encoding(resp.headers_mut());

  if let Some(len) = resp
    .headers()
    .get(CONTENT_LENGTH)
    .and_then(|v| v.to_str().ok())
    .and_then(|v| v.parse::<usize>().ok())
    && len < cfg.min_size
  {
    return resp;
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
    resp.headers_mut().remove(ACCEPT_RANGES);
  }

  resp
}

fn negotiate(req: &Request, cfg: &Config) -> Option<Encoding> {
  let accepted = req
    .headers()
    .get(ACCEPT_ENCODING)
    .and_then(|v| v.to_str().ok())
    .unwrap_or("");
  choose_encoding(accepted, &cfg.enabled)
}

fn eligible(resp: &Response, cfg: &Config, request_is_authenticated: bool) -> bool {
  let status = resp.status();
  let headers = resp.headers();
  status.is_success()
    && !matches!(
      status,
      StatusCode::NO_CONTENT | StatusCode::RESET_CONTENT | StatusCode::PARTIAL_CONTENT
    )
    && !headers.contains_key(CONTENT_RANGE)
    && !headers.contains_key(CONTENT_ENCODING)
    // CRIME/BREACH: compressing secrets beside attacker-controlled content leaks them
    // through the compressed length.
    && !(cfg.protect_sensitive
      && (request_is_authenticated || headers.contains_key(http::header::SET_COOKIE)))
    && headers
      .get(CONTENT_TYPE)
      .is_none_or(|ct| cfg.content_types.matches(ct.to_str().unwrap_or("")))
}

fn is_event_stream(resp: &Response) -> bool {
  resp
    .headers()
    .get(CONTENT_TYPE)
    .and_then(|ct| ct.to_str().ok())
    .and_then(|ct| ct.split(';').next())
    .is_some_and(|mime| mime.trim().eq_ignore_ascii_case("text/event-stream"))
}

/// Deliberately broad: any auth header or cookie marks the response as a CRIME/BREACH target.
fn request_carries_credentials(req: &Request) -> bool {
  req.headers().contains_key(http::header::AUTHORIZATION)
    || req
      .headers()
      .contains_key(http::header::PROXY_AUTHORIZATION)
    || req.headers().contains_key(http::header::COOKIE)
}

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
