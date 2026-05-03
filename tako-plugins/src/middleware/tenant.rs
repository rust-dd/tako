//! Multi-tenant middleware: extracts a tenant identifier from the request
//! and exposes it to handlers via the [`Tenant`] extension.
//!
//! Strategies:
//!
//! - `Header(name)` — read a fixed header value (default: `X-Tenant-ID`).
//! - `Subdomain` — peel the leading label off the `Host` header
//!   (`acme.example.com` → `acme`).
//! - `PathPrefix(pos)` — pick a positional path segment (`/t/{id}/...`).
//! - `Custom(fn)` — handler-supplied closure for hybrid extraction.
//!
//! When extraction fails, the middleware optionally rejects with `400 Bad
//! Request`. Otherwise the extension is simply absent.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderName;
use http::StatusCode;
use tako_core::body::TakoBody;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::types::Request;
use tako_core::types::Response;

/// Decoded tenant identifier inserted into request extensions.
#[derive(Debug, Clone)]
pub struct Tenant(pub String);

/// Tenant extraction strategy.
#[derive(Clone)]
pub enum TenantStrategy {
  /// Read a fixed header.
  Header(HeaderName),
  /// First label of the `Host` header.
  Subdomain,
  /// Zero-based path segment index (e.g. `1` selects `acme` in `/t/acme/...`).
  PathPrefix(usize),
  /// Caller-defined closure.
  Custom(TenantCustomFn),
}

/// Closure that extracts a tenant identifier from the request.
pub type TenantCustomFn = Arc<dyn Fn(&Request) -> Option<String> + Send + Sync + 'static>;

/// Multi-tenant middleware.
pub struct TenantMiddleware {
  strategy: TenantStrategy,
  required: bool,
}

impl TenantMiddleware {
  /// Header-based extraction (default header name is `X-Tenant-ID`).
  pub fn from_header(name: HeaderName) -> Self {
    Self {
      strategy: TenantStrategy::Header(name),
      required: false,
    }
  }

  /// Subdomain-based extraction.
  pub fn from_subdomain() -> Self {
    Self {
      strategy: TenantStrategy::Subdomain,
      required: false,
    }
  }

  /// Path-segment-based extraction.
  pub fn from_path_segment(index: usize) -> Self {
    Self {
      strategy: TenantStrategy::PathPrefix(index),
      required: false,
    }
  }

  /// Custom closure-based extraction.
  pub fn custom<F>(f: F) -> Self
  where
    F: Fn(&Request) -> Option<String> + Send + Sync + 'static,
  {
    Self {
      strategy: TenantStrategy::Custom(Arc::new(f)),
      required: false,
    }
  }

  /// When set, requests without a tenant identifier are rejected with 400.
  pub fn require(mut self, required: bool) -> Self {
    self.required = required;
    self
  }
}

fn extract_subdomain(host: &str) -> Option<String> {
  // Drop port if present (`acme.example.com:8080`).
  let host = host.split(':').next().unwrap_or(host);
  let mut labels = host.split('.');
  let first = labels.next()?;
  // Need at least one further label so we don't return the whole apex domain.
  labels.next()?;
  if first.is_empty() {
    return None;
  }
  Some(first.to_ascii_lowercase())
}

fn extract_path_segment(path: &str, index: usize) -> Option<String> {
  path
    .split('/')
    .filter(|s| !s.is_empty())
    .nth(index)
    .map(str::to_string)
}

impl IntoMiddleware for TenantMiddleware {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let strategy = self.strategy;
    let required = self.required;

    move |mut req: Request, next: Next| {
      let strategy = strategy.clone();
      Box::pin(async move {
        let tenant = match &strategy {
          TenantStrategy::Header(h) => req
            .headers()
            .get(h)
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
          TenantStrategy::Subdomain => req
            .headers()
            .get(http::header::HOST)
            .and_then(|v| v.to_str().ok())
            .and_then(extract_subdomain),
          TenantStrategy::PathPrefix(idx) => extract_path_segment(req.uri().path(), *idx),
          TenantStrategy::Custom(f) => f(&req),
        };

        match tenant {
          Some(t) => {
            req.extensions_mut().insert(Tenant(t));
            next.run(req).await
          }
          None if required => http::Response::builder()
            .status(StatusCode::BAD_REQUEST)
            .body(TakoBody::from("missing tenant identifier"))
            .expect("valid response"),
          None => next.run(req).await,
        }
      })
    }
  }
}
