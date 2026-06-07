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
  constraints: VerifyConstraints,
  revocation: Option<RevocationCheck<V::Claims>>,
  introspect: Option<IntrospectionFn>,
}

impl<V: JwtVerifier> JwtAuth<V> {
  /// Creates a JWT auth middleware with the given verifier and no extra
  /// constraints / revocation.
  pub fn new(verifier: V) -> Self {
    Self {
      verifier,
      constraints: VerifyConstraints::default(),
      revocation: None,
      introspect: None,
    }
  }

  /// Sets per-claim constraints (issuer, audience, leeway).
  pub fn constraints(mut self, c: VerifyConstraints) -> Self {
    self.constraints = c;
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
    let verifier = self.verifier;
    let constraints = Arc::new(self.constraints);
    let revocation = self.revocation;
    let introspect = self.introspect;

    move |mut req: Request, next: Next| {
      let verifier = verifier.clone();
      let constraints = constraints.clone();
      let revocation = revocation.clone();
      let introspect = introspect.clone();

      Box::pin(async move {
        // PMW-04: RFC 7235 §2.1 requires the auth scheme name to be
        // matched case-insensitively. Sibling `bearer_auth.rs:205` already
        // uses `eq_ignore_ascii_case`; here we previously used the
        // case-sensitive `strip_prefix("Bearer ")` which silently 401'd
        // any legitimate `bearer <jwt>` / `BEARER <jwt>` client.
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

        let claims = match verifier.verify(&token) {
          Ok(c) => c,
          Err(e) => {
            return (StatusCode::UNAUTHORIZED, format!("Invalid token: {e}")).into_response();
          }
        };

        // Caller-controlled iss/aud/leeway. Propagate to the verifier so it
        // can apply them. Default trait impl fails closed when constraints
        // are configured but the verifier does not implement enforcement.
        if let Err(e) = verifier.validate_constraints(&claims, &constraints) {
          return (StatusCode::UNAUTHORIZED, format!("Invalid token: {e}")).into_response();
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
