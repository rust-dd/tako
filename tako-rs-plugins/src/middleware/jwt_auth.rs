//! JWT (JSON Web Token) authentication middleware.
//!
//! Trait-based: implement [`JwtVerifier`] with your preferred JWT library
//! and pass it to [`JwtAuth`]. Enable the `jwt-simple` cargo feature for the
//! batteries-included verifier built on top of `jwt-simple` — it supports
//! HMAC, RSA, RSA-PSS, ECDSA, `EdDSA` and `BLAKE2b`.
//!
//! v2 additions:
//!
//! - **JWKS rotation** via [`stores::JwksProvider`](crate::stores::JwksProvider).
//!   The bundled `MultiKeyVerifier` (under the `jwt-simple` feature) selects keys by `kid`, falling back to
//!   the configured static map when the provider returns no match.
//! - **Configurable issuer / audience / leeway** through
//!   [`VerifyConstraints`]. Applied uniformly across every algorithm.
//! - **Revocation list** via the [`RevocationList`] trait — simple in-memory
//!   `HashSet<String>` of revoked `jti` values is provided.
//! - **Optional remote introspection** via [`IntrospectionFn`] — the
//!   middleware calls back on every request when configured, which is the
//!   correct hook for opaque tokens or tenant-scoped revocation.

#[cfg(feature = "jwt-simple")]
mod jwt_simple;
mod layer;
mod revocation;
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
