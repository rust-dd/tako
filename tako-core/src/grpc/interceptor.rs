//! gRPC-specific interceptor middleware.
//!
//! HTTP middleware (`tako_core::middleware::Next`) operates on `Request` /
//! `Response`, so it can pre/post-process arbitrary HTTP traffic. gRPC adds
//! two extra concerns that don't translate cleanly:
//!
//! 1. **Status lives in trailers** — modifying `grpc-status` from a regular
//!    middleware requires rewriting the response body's trailing frame.
//! 2. **Per-method interceptors** — gRPC interceptors are usually attached
//!    per-service or per-method, not per-HTTP-route.
//!
//! This module exposes:
//!
//! - [`GrpcInterceptor`] — async trait whose `intercept` runs before the
//!   handler with a typed view of metadata; can short-circuit with a
//!   [`super::GrpcStatus`] error.
//! - [`InterceptorChain`] — sequence of interceptors run in order. Wraps a
//!   gRPC handler so the call surface is uniform.
//!
//! ⚠️ **Status:** wiring into `Router::route` will land alongside the
//! generated stub story. The traits below already let plugin code build
//! standalone gRPC handlers that respect interceptors.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderMap;

use super::GrpcStatus;
use crate::types::Request;
use crate::types::Response;

/// gRPC interceptor — runs before a gRPC handler.
pub trait GrpcInterceptor: Send + Sync + 'static {
  /// Inspect or modify metadata; return `Err(GrpcStatus)` to short-circuit.
  fn intercept(
    &self,
    metadata: &mut HeaderMap,
    method: &str,
  ) -> Pin<Box<dyn Future<Output = Result<(), GrpcStatus>> + Send + '_>>;
}

/// Ordered list of interceptors that must all succeed.
#[derive(Default, Clone)]
pub struct InterceptorChain {
  inner: Vec<Arc<dyn GrpcInterceptor>>,
}

impl InterceptorChain {
  /// Empty chain.
  pub fn new() -> Self {
    Self::default()
  }

  /// Append an interceptor.
  pub fn push<I: GrpcInterceptor + 'static>(mut self, interceptor: I) -> Self {
    self.inner.push(Arc::new(interceptor));
    self
  }

  /// Run all interceptors against `metadata` for `method`. Returns the first
  /// failure or `Ok(())` if every interceptor succeeded.
  pub async fn run(&self, metadata: &mut HeaderMap, method: &str) -> Result<(), GrpcStatus> {
    for ic in &self.inner {
      ic.intercept(metadata, method).await?;
    }
    Ok(())
  }
}

/// Helper that bundles a chain with a request-derived gRPC method name.
///
/// Plugin code building a gRPC service can call this from the entry point of
/// each method handler to apply the chain uniformly.
pub async fn run_chain(
  chain: &InterceptorChain,
  req: &mut Request,
  method: &str,
) -> Result<(), Response> {
  let metadata = req.headers_mut();
  match chain.run(metadata, method).await {
    Ok(()) => Ok(()),
    Err(status) => Err(super::build_grpc_error_response(
      status.code,
      status.message.as_deref().unwrap_or(""),
    )),
  }
}
