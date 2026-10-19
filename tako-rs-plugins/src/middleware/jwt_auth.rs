//! JWT authentication over a pluggable [`JwtVerifier`] (the `jwt-simple` feature bundles one),
//! with JWKS rotation, issuer/audience constraints, revocation and remote introspection.

#[cfg(feature = "jwt-simple")]
mod jwt_simple;
mod layer;
#[cfg(feature = "jwt-simple")]
mod provider_key;
mod revocation;
mod rotation;
mod verifier;

#[cfg(feature = "jwt-simple")]
pub use jwt_simple::AnyVerifyKey;
#[cfg(feature = "jwt-simple")]
pub use jwt_simple::MultiKeyVerifier;
pub use layer::JwtAuth;
pub use revocation::InMemoryRevocationList;
pub use revocation::IntrospectionFn;
pub use revocation::JtiExtractorFn;
pub use revocation::RevocationCheck;
pub use revocation::RevocationList;
pub use verifier::ConstraintsNotSupported;
pub use verifier::JwtVerifier;
pub use verifier::VerifyConstraints;

#[cfg(all(test, feature = "jwt-simple"))]
mod tests;
