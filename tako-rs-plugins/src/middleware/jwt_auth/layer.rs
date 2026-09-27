//! The [`JwtAuth`] middleware layer and its request-time enforcement.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use http::header::AUTHORIZATION;
use tako_rs_core::middleware::IntoMiddleware;
use tako_rs_core::middleware::Next;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;
use tako_rs_core::types::Response;

use super::revocation::IntrospectionFn;
use super::revocation::RevocationCheck;
use super::revocation::RevocationList;
use super::verifier::JwtVerifier;
use super::verifier::VerifyConstraints;

/// JWT authentication middleware.
pub struct JwtAuth<V: JwtVerifier> {
  verifier: V,
  constraints: Option<VerifyConstraints>,
  revocation: Option<RevocationCheck<V::Claims>>,
  introspect: Option<IntrospectionFn>,
  provider: Option<Arc<dyn crate::stores::JwksProvider>>,
}

impl<V: JwtVerifier> JwtAuth<V> {
  /// Creates a JWT auth middleware with the given verifier and no extra
  /// constraints / revocation.
  pub fn new(verifier: V) -> Self {
    Self {
      verifier,
      constraints: None,
      revocation: None,
      introspect: None,
      provider: None,
    }
  }

  /// Use an asynchronous key provider. The verifier must implement `verify_with_key`.
  pub fn store(mut self, provider: impl crate::stores::JwksProvider) -> Self {
    self.provider = Some(Arc::new(provider));
    self
  }

  /// Sets per-claim constraints (issuer, audience, leeway).
  pub fn constraints(mut self, c: VerifyConstraints) -> Self {
    self.constraints = Some(c);
    self
  }

  /// Plugs a revocation list checked after signature verification.
  /// `extractor` returns the revocation key (typically the `jti` claim) for
  /// each decoded claims value.
  pub fn revocation<R, F>(mut self, list: R, extractor: F) -> Self
  where
    R: RevocationList,
    F: Fn(&V::Claims) -> Option<String> + Send + Sync + 'static,
  {
    self.revocation = Some((Arc::new(list), Arc::new(extractor)));
    self
  }

  /// Plugs a remote introspection callback. The callback is invoked on every
  /// successful local verification — short-lived caches belong inside the
  /// callback itself.
  pub fn introspect<F, Fut>(mut self, f: F) -> Self
  where
    F: Fn(&str) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = bool> + Send + 'static,
  {
    self.introspect = Some(Arc::new(move |t: &str| Box::pin(f(t))));
    self
  }
}

impl<V: JwtVerifier> IntoMiddleware for JwtAuth<V> {
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let verifier = self.constraints.as_ref().map_or_else(
      || self.verifier.clone(),
      |constraints| self.verifier.with_constraints(constraints),
    );
    let provider = self.provider;
    let constraints = Arc::new(self.constraints.unwrap_or_default());
    let revocation = self.revocation;
    let introspect = self.introspect;

    move |mut req: Request, next: Next| {
      let verifier = verifier.clone();
      let provider = provider.clone();
      let constraints = constraints.clone();
      let revocation = revocation.clone();
      let introspect = introspect.clone();

      Box::pin(async move {
        let token = match req
          .headers()
          .get(AUTHORIZATION)
          .and_then(|v| v.to_str().ok())
          .and_then(|s| s.split_once(' '))
          .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("Bearer"))
          .map(|(_, rest)| rest.trim())
        {
          Some(t) => t.to_string(),
          None => {
            return (
              StatusCode::UNAUTHORIZED,
              "Missing or invalid Authorization header",
            )
              .into_response();
          }
        };

        let claims = match super::rotation::verify(&verifier, &token, provider.as_deref()).await {
          Ok(Some(claims)) => claims,
          Ok(None) => return unauthorized(),
          Err(error) => return crate::stores::runtime::unavailable(error),
        };

        if let Err(e) = verifier.validate_constraints(&claims, &constraints) {
          tracing::debug!(error = %e, "JWT constraints rejected");
          return unauthorized();
        }

        if let Some((list, extractor)) = revocation.as_ref()
          && let Some(jti) = extractor(&claims)
          && list.is_revoked(&jti)
        {
          return (StatusCode::UNAUTHORIZED, "token revoked").into_response();
        }

        if let Some(introspect) = introspect.as_ref()
          && !introspect(&token).await
        {
          return (StatusCode::UNAUTHORIZED, "token introspection failed").into_response();
        }

        req.extensions_mut().insert(claims);
        next.run(req).await.into_response()
      })
    }
  }
}

fn unauthorized() -> Response {
  http::Response::builder()
    .status(StatusCode::UNAUTHORIZED)
    .header(http::header::WWW_AUTHENTICATE, "Bearer")
    .body(tako_rs_core::body::TakoBody::from("Invalid token"))
    .expect("valid JWT rejection")
}
