//! JWT (JSON Web Token) authentication middleware.
//!
//! This module provides a trait-based JWT authentication middleware. Implement
//! [`JwtVerifier`] with your preferred JWT library and pass it to [`JwtAuth`].
//!
//! Enable the `jwt-simple` feature for a batteries-included implementation
//! via [`AnyVerifyKey`] that supports HMAC, RSA, ECDSA, and EdDSA algorithms.
//!
//! # Examples
//!
//! ## Custom verifier (no extra dependency)
//!
//! ```rust,ignore
//! use tako::middleware::jwt_auth::{JwtAuth, JwtVerifier};
//! use tako::middleware::IntoMiddleware;
//!
//! #[derive(Clone)]
//! struct MyVerifier { /* your key */ }
//!
//! impl JwtVerifier for MyVerifier {
//!     type Claims = MyClaims;
//!     type Error = MyError;
//!
//!     fn verify(&self, token: &str) -> Result<Self::Claims, Self::Error> {
//!         // your verification logic
//!         todo!()
//!     }
//! }
//!
//! let auth = JwtAuth::new(MyVerifier { /* ... */ });
//! let middleware = auth.into_middleware();
//! ```
//!
//! ## With `jwt-simple` feature
//!
//! ```rust,ignore
//! use tako::middleware::jwt_auth::{JwtAuth, AnyVerifyKey};
//! use tako::middleware::IntoMiddleware;
//! use jwt_simple::prelude::*;
//! use std::collections::HashMap;
//!
//! let key = HS256Key::generate();
//! let mut keys = HashMap::new();
//! keys.insert("HS256", AnyVerifyKey::HS256(std::sync::Arc::new(key)));
//!
//! let auth = JwtAuth::new(keys);
//! let middleware = auth.into_middleware();
//! ```

use std::fmt;
use std::future::Future;
use std::pin::Pin;

use http::StatusCode;
use http::header::AUTHORIZATION;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;

/// Trait for verifying JWT tokens.
///
/// Implement this trait with your preferred JWT library to plug into [`JwtAuth`].
pub trait JwtVerifier: Send + Sync + Clone + 'static {
  /// The decoded claims type that will be inserted into request extensions.
  type Claims: Send + Sync + Clone + 'static;
  /// The error type returned when verification fails.
  type Error: fmt::Display;

  /// Verify a raw JWT token string and return the decoded claims.
  fn verify(&self, token: &str) -> Result<Self::Claims, Self::Error>;
}

/// JWT authentication middleware.
///
/// Extracts a Bearer token from the `Authorization` header, verifies it using
/// the provided [`JwtVerifier`], and injects the decoded claims into request
/// extensions for downstream handlers.
pub struct JwtAuth<V: JwtVerifier> {
  verifier: V,
}

impl<V: JwtVerifier> JwtAuth<V> {
  /// Creates a new JWT authentication middleware with the given verifier.
  pub fn new(verifier: V) -> Self {
    Self { verifier }
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

    move |mut req: Request, next: Next| {
      let verifier = verifier.clone();

      Box::pin(async move {
        let token = match req
          .headers()
          .get(AUTHORIZATION)
          .and_then(|v| v.to_str().ok())
          .and_then(|s| s.strip_prefix("Bearer "))
          .map(str::trim)
        {
          Some(t) => t,
          None => {
            return (
              StatusCode::UNAUTHORIZED,
              "Missing or invalid Authorization header",
            )
              .into_response();
          }
        };

        let claims = match verifier.verify(token) {
          Ok(c) => c,
          Err(e) => {
            return (StatusCode::UNAUTHORIZED, format!("Invalid token: {e}")).into_response();
          }
        };

        req.extensions_mut().insert(claims);
        next.run(req).await.into_response()
      })
    }
  }
}

#[cfg(feature = "jwt-simple")]
mod jwt_simple_impl {
  use std::collections::HashMap;
  use std::sync::Arc;

  use ::jwt_simple::prelude::*;
  use serde::Serialize;
  use serde::de::DeserializeOwned;
  use tako_core::types::BuildHasher;

  use super::*;

  /// Multi-algorithm JWT verification key wrapper.
  ///
  /// Supports HMAC, RSA, RSA-PSS, ECDSA, and EdDSA algorithms.
  /// Use with [`JwtAuth`] for batteries-included JWT authentication.
  pub enum AnyVerifyKey {
    /// HMAC-SHA256 symmetric key.
    HS256(Arc<HS256Key>),
    /// HMAC-SHA384 symmetric key.
    HS384(Arc<HS384Key>),
    /// HMAC-SHA512 symmetric key.
    HS512(Arc<HS512Key>),
    /// BLAKE2b symmetric key.
    Blake2b(Arc<Blake2bKey>),

    /// RSA-SHA256 public key (PKCS#1 v1.5).
    RS256(Arc<RS256PublicKey>),
    /// RSA-SHA384 public key (PKCS#1 v1.5).
    RS384(Arc<RS384PublicKey>),
    /// RSA-SHA512 public key (PKCS#1 v1.5).
    RS512(Arc<RS512PublicKey>),

    /// RSA-SHA256 public key (PSS).
    PS256(Arc<PS256PublicKey>),
    /// RSA-SHA384 public key (PSS).
    PS384(Arc<PS384PublicKey>),
    /// RSA-SHA512 public key (PSS).
    PS512(Arc<PS512PublicKey>),

    /// ECDSA P-256 / SHA-256.
    ES256(Arc<ES256PublicKey>),
    /// ECDSA secp256k1 / SHA-256.
    ES256K(Arc<ES256kPublicKey>),
    /// ECDSA P-384 / SHA-384.
    ES384(Arc<ES384PublicKey>),

    /// Ed25519.
    EdDSA(Arc<Ed25519PublicKey>),
  }

  impl AnyVerifyKey {
    /// Returns the algorithm identifier for this key.
    pub fn alg_id(&self) -> &'static str {
      match self {
        Self::HS256(_) => "HS256",
        Self::HS384(_) => "HS384",
        Self::HS512(_) => "HS512",
        Self::Blake2b(_) => "BLAKE2B",
        Self::RS256(_) => "RS256",
        Self::RS384(_) => "RS384",
        Self::RS512(_) => "RS512",
        Self::PS256(_) => "PS256",
        Self::PS384(_) => "PS384",
        Self::PS512(_) => "PS512",
        Self::ES256(_) => "ES256",
        Self::ES256K(_) => "ES256K",
        Self::ES384(_) => "ES384",
        Self::EdDSA(_) => "EdDSA",
      }
    }

    fn verify_token<C>(&self, token: &str) -> Result<JWTClaims<C>, ::jwt_simple::Error>
    where
      C: Serialize + DeserializeOwned,
    {
      let opts = VerificationOptions::default();
      match self {
        Self::HS256(k) => k.verify_token::<C>(token, Some(opts)),
        Self::HS384(k) => k.verify_token::<C>(token, Some(opts)),
        Self::HS512(k) => k.verify_token::<C>(token, Some(opts)),
        Self::Blake2b(k) => k.verify_token::<C>(token, Some(opts)),
        Self::RS256(k) => k.verify_token::<C>(token, Some(opts)),
        Self::RS384(k) => k.verify_token::<C>(token, Some(opts)),
        Self::RS512(k) => k.verify_token::<C>(token, Some(opts)),
        Self::PS256(k) => k.verify_token::<C>(token, Some(opts)),
        Self::PS384(k) => k.verify_token::<C>(token, Some(opts)),
        Self::PS512(k) => k.verify_token::<C>(token, Some(opts)),
        Self::ES256(k) => k.verify_token::<C>(token, Some(opts)),
        Self::ES256K(k) => k.verify_token::<C>(token, Some(opts)),
        Self::ES384(k) => k.verify_token::<C>(token, Some(opts)),
        Self::EdDSA(k) => k.verify_token::<C>(token, Some(opts)),
      }
    }
  }

  impl Clone for AnyVerifyKey {
    fn clone(&self) -> Self {
      match self {
        Self::HS256(k) => Self::HS256(Arc::clone(k)),
        Self::HS384(k) => Self::HS384(Arc::clone(k)),
        Self::HS512(k) => Self::HS512(Arc::clone(k)),
        Self::Blake2b(k) => Self::Blake2b(Arc::clone(k)),
        Self::RS256(k) => Self::RS256(Arc::clone(k)),
        Self::RS384(k) => Self::RS384(Arc::clone(k)),
        Self::RS512(k) => Self::RS512(Arc::clone(k)),
        Self::PS256(k) => Self::PS256(Arc::clone(k)),
        Self::PS384(k) => Self::PS384(Arc::clone(k)),
        Self::PS512(k) => Self::PS512(Arc::clone(k)),
        Self::ES256(k) => Self::ES256(Arc::clone(k)),
        Self::ES256K(k) => Self::ES256K(Arc::clone(k)),
        Self::ES384(k) => Self::ES384(Arc::clone(k)),
        Self::EdDSA(k) => Self::EdDSA(Arc::clone(k)),
      }
    }
  }

  /// Multi-algorithm verifier backed by `jwt-simple`.
  ///
  /// Wraps a map of algorithm identifiers to [`AnyVerifyKey`] instances. The token
  /// header is decoded to determine the algorithm, then the matching key is used.
  pub struct MultiKeyVerifier<C> {
    keys: HashMap<&'static str, AnyVerifyKey, BuildHasher>,
    _phantom: std::marker::PhantomData<C>,
  }

  impl<C> Clone for MultiKeyVerifier<C> {
    fn clone(&self) -> Self {
      Self {
        keys: self.keys.clone(),
        _phantom: std::marker::PhantomData,
      }
    }
  }

  impl<C> MultiKeyVerifier<C> {
    /// Creates a new multi-key verifier from a map of algorithm names to keys.
    pub fn new(keys: HashMap<&'static str, AnyVerifyKey, BuildHasher>) -> Self {
      Self {
        keys,
        _phantom: std::marker::PhantomData,
      }
    }
  }

  impl<C> JwtVerifier for MultiKeyVerifier<C>
  where
    C: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
  {
    type Claims = JWTClaims<C>;
    type Error = String;

    fn verify(&self, token: &str) -> Result<Self::Claims, Self::Error> {
      let meta = ::jwt_simple::token::Token::decode_metadata(token)
        .map_err(|e| format!("Cannot decode JWT header: {e}"))?;

      let alg = meta.algorithm();
      let key = self
        .keys
        .get(alg)
        .ok_or_else(|| format!("Algorithm {alg} not allowed"))?;

      key.verify_token::<C>(token).map_err(|e| e.to_string())
    }
  }
}

#[cfg(feature = "jwt-simple")]
pub use jwt_simple_impl::AnyVerifyKey;
#[cfg(feature = "jwt-simple")]
pub use jwt_simple_impl::MultiKeyVerifier;
