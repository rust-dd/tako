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

  /// Verify against a provider-supplied key, including an explicit algorithm allow-list.
  /// The default does not support external keys and returns `None`, which fails closed.
  fn verify_with_key(
    &self,
    _token: &str,
    _key: &crate::stores::VerificationKey,
  ) -> Option<Result<Self::Claims, Self::Error>> {
    None
  }

  /// Configure constraints before signature and time validation.
  /// The default returns a clone; unsupported constraints fail in `validate_constraints`.
  fn with_constraints(&self, _constraints: &VerifyConstraints) -> Self {
    self.clone()
  }

  /// Enforce middleware constraints. Non-default constraints fail closed by default.
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
  /// Diagnostic logged when verification is rejected.
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
