//! JWT verification contract and constraint configuration.

use std::fmt;

/// Trait for verifying JWT tokens.
pub trait JwtVerifier: Send + Sync + Clone + 'static {
  /// Decoded claims inserted into request extensions.
  type Claims: Send + Sync + Clone + 'static;
  /// Verification error.
  type Error: fmt::Display;

  /// Verifies a raw JWT token string.
  fn verify(&self, token: &str) -> Result<Self::Claims, Self::Error>;

  /// Validate `iss` / `aud` / `leeway` constraints against the decoded claims.
  ///
  /// The default implementation **fails closed** when any non-default
  /// constraint is configured — concrete verifiers MUST override this if they
  /// want to silently accept (because they already enforce constraints
  /// internally) or to apply their own logic. Failing closed prevents the
  /// previous v1.x behavior where custom verifiers silently dropped the
  /// `VerifyConstraints` configured on `JwtAuth`, leaving iss/aud/leeway
  /// unenforced.
  fn validate_constraints(
    &self,
    _claims: &Self::Claims,
    constraints: &VerifyConstraints,
  ) -> Result<(), ConstraintsNotSupported> {
    if constraints.issuer.is_some()
      || constraints.audience.is_some()
      || constraints.leeway_secs != 0
    {
      Err(ConstraintsNotSupported {
        reason: "this JwtVerifier does not override `validate_constraints`; \
                 configure constraints on the verifier itself or implement \
                 `validate_constraints` on your custom verifier",
      })
    } else {
      Ok(())
    }
  }
}

/// Reported by [`JwtVerifier::validate_constraints`] when the verifier cannot
/// (or won't) enforce the requested `VerifyConstraints`. The middleware
/// surfaces this as 401 Unauthorized — fail-closed by design.
#[derive(Debug, Clone)]
pub struct ConstraintsNotSupported {
  /// Human-readable diagnostic surfaced in the 401 response body.
  pub reason: &'static str,
}

impl fmt::Display for ConstraintsNotSupported {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "constraints not enforceable: {}", self.reason)
  }
}

/// Optional global verification constraints applied on top of the verifier.
#[derive(Default, Clone)]
pub struct VerifyConstraints {
  /// Required issuer (`iss` claim).
  pub issuer: Option<String>,
  /// Required audience (`aud` claim).
  pub audience: Option<String>,
  /// Allowed clock skew in seconds.
  pub leeway_secs: u64,
}
