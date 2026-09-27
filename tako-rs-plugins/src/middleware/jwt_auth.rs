//! JWT (JSON Web Token) authentication middleware.
//!
//! Trait-based: implement [`JwtVerifier`] with your preferred JWT library
//! and pass it to [`JwtAuth`]. Enable the `jwt-simple` cargo feature for the
//! batteries-included verifier built on top of `jwt-simple` — it supports
//! HMAC, RSA, RSA-PSS, ECDSA, `EdDSA` and `BLAKE2b`.
//!
//! - **JWKS rotation** via [`stores::JwksProvider`](crate::stores::JwksProvider).
//!   Attach a provider with `JwtAuth::store`. The bundled `MultiKeyVerifier`
//!   supports raw MAC keys and DER public keys; its algorithm allow-list also
//!   applies to provider keys. An empty provider result uses static keys.
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
