//! gRPC interceptors: checks on request metadata that run before a handler and can end the
//! call with a [`GrpcStatus`].

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::HeaderMap;

use super::GrpcStatus;
use crate::responder::Responder;
use crate::types::Request;
use crate::types::Response;

/// Runs before a gRPC handler, see [`run_chain`].
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
  pub fn new() -> Self {
    Self::default()
  }

  pub fn push<I: GrpcInterceptor + 'static>(mut self, interceptor: I) -> Self {
    self.inner.push(Arc::new(interceptor));
    self
  }

  /// Runs the interceptors in order, stopping at the first failure.
  pub async fn run(&self, metadata: &mut HeaderMap, method: &str) -> Result<(), GrpcStatus> {
    for ic in &self.inner {
      ic.intercept(metadata, method).await?;
    }
    Ok(())
  }
}

/// Runs `chain` on the request headers; a failure comes back as a trailers-only response.
#[allow(
  clippy::result_large_err,
  reason = "Boxing the response would break the public interceptor API"
)]
pub async fn run_chain(
  chain: &InterceptorChain,
  req: &mut Request,
  method: &str,
) -> Result<(), Response> {
  chain
    .run(req.headers_mut(), method)
    .await
    .map_err(GrpcStatus::into_response)
}
