//! JWT (JSON Web Token) authentication middleware with multi-algorithm support.
//!
//! This module provides comprehensive JWT authentication middleware supporting multiple
//! cryptographic algorithms including HMAC, RSA, ECDSA, and EdDSA. The middleware validates
//! JWT tokens from Authorization headers, verifies signatures using configured keys, and
//! injects decoded claims into request extensions for downstream handlers. It integrates
//! with the jwt-simple crate for robust token processing and validation.
//!
//! # Examples
//!
//! ```rust
//! use tako::middleware::jwt_auth::{JwtAuth, AnyVerifyKey};
//! use tako::middleware::IntoMiddleware;
//! use jwt_simple::prelude::*;
//! use serde::{Deserialize, Serialize};
//! use std::collections::HashMap;
//!
//! #[derive(Serialize, Deserialize, Clone)]
//! struct UserClaims {
//!     user_id: u32,
//!     role: String,
//!     exp: u64,
//! }
//!
//! // HMAC-based JWT authentication
//! let hmac_key = HS256Key::generate();
//! let mut keys = HashMap::new();
//! keys.insert("HS256", AnyVerifyKey::HS256(std::sync::Arc::new(hmac_key)));
//!
//! let jwt_auth = JwtAuth::<UserClaims>::new(keys);
//! let middleware = jwt_auth.into_middleware();
//!
//! // Multiple algorithm support
//! let mut multi_keys = HashMap::new();
//! multi_keys.insert("HS256", AnyVerifyKey::HS256(std::sync::Arc::new(HS256Key::generate())));
//! multi_keys.insert("RS256", AnyVerifyKey::RS256(std::sync::Arc::new(
//!     RS256PublicKey::from_pem("-----BEGIN PUBLIC KEY-----...").unwrap()
//! )));
//!
//! let multi_auth = JwtAuth::<UserClaims>::new(multi_keys);
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use http::header::AUTHORIZATION;
use jwt_simple::prelude::*;
use serde::de::DeserializeOwned;

use crate::middleware::IntoMiddleware;
use crate::middleware::Next;
use crate::responder::Responder;
use crate::types::BuildHasher;
use crate::types::Request;
use crate::types::Response;

/// Multi-algorithm JWT verification key wrapper supporting various cryptographic algorithms.
///
/// `AnyVerifyKey` provides a unified interface for different JWT signing algorithms,
/// allowing applications to support multiple key types simultaneously. This enables
/// flexible JWT validation scenarios such as key rotation, multi-tenant applications,
/// or supporting legacy and modern algorithms concurrently.
///
/// # Supported Algorithms
///
/// - **HMAC**: HS256, HS384, HS512, BLAKE2B - Symmetric key algorithms
/// - **RSA**: RS256, RS384, RS512 - RSA signatures with PKCS#1 v1.5 padding
/// - **RSA-PSS**: PS256, PS384, PS512 - RSA signatures with PSS padding
/// - **ECDSA**: ES256, ES256K, ES384 - Elliptic curve signatures
/// - **EdDSA**: Ed25519 - Edwards curve signatures
///
/// # Examples
///
/// ```rust
/// use tako::middleware::jwt_auth::AnyVerifyKey;
/// use jwt_simple::prelude::*;
/// use std::sync::Arc;
///
/// // HMAC key
/// let hmac_key = HS256Key::generate();
/// let verify_key = AnyVerifyKey::HS256(Arc::new(hmac_key));
/// assert_eq!(verify_key.alg_id(), "HS256");
///
/// // RSA public key
/// let rsa_key = RS256PublicKey::from_pem("-----BEGIN PUBLIC KEY-----...").unwrap();
/// let rsa_verify = AnyVerifyKey::RS256(Arc::new(rsa_key));
/// assert_eq!(rsa_verify.alg_id(), "RS256");
/// ```
pub enum AnyVerifyKey {
  /// HMAC-SHA256 symmetric key.
  HS256(Arc<HS256Key>),
  /// HMAC-SHA384 symmetric key.
  HS384(Arc<HS384Key>),
  /// HMAC-SHA512 symmetric key.
  HS512(Arc<HS512Key>),
  /// BLAKE2b symmetric key for high-performance hashing.
  Blake2b(Arc<Blake2bKey>),

  /// RSA-SHA256 public key with PKCS#1 v1.5 padding.
  RS256(Arc<RS256PublicKey>),
  /// RSA-SHA384 public key with PKCS#1 v1.5 padding.
  RS384(Arc<RS384PublicKey>),
  /// RSA-SHA512 public key with PKCS#1 v1.5 padding.
  RS512(Arc<RS512PublicKey>),

  /// RSA-SHA256 public key with PSS padding.
  PS256(Arc<PS256PublicKey>),
  /// RSA-SHA384 public key with PSS padding.
  PS384(Arc<PS384PublicKey>),
  /// RSA-SHA512 public key with PSS padding.
  PS512(Arc<PS512PublicKey>),

  /// ECDSA with P-256 curve and SHA-256.
  ES256(Arc<ES256PublicKey>),
  /// ECDSA with secp256k1 curve and SHA-256.
  ES256K(Arc<ES256kPublicKey>),
  /// ECDSA with P-384 curve and SHA-384.
  ES384(Arc<ES384PublicKey>),

  /// Ed25519 Edwards curve signature.
  EdDSA(Arc<Ed25519PublicKey>),
}

impl AnyVerifyKey {
  /// Returns the algorithm identifier for this verification key.
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

  /// Verifies a JWT token using this key and returns the decoded claims.
  pub fn verify<C>(&self, token: &str) -> Result<JWTClaims<C>, jwt_simple::Error>
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

/// JWT authentication middleware configuration with multi-algorithm support.
///
/// `JwtAuth` provides middleware for validating JWT tokens with support for multiple
/// cryptographic algorithms simultaneously. It extracts tokens from Authorization headers,
/// validates them against configured verification keys, and injects decoded claims into
/// request extensions for use by downstream handlers.
///
/// # Type Parameters
///
/// * `T` - Claims type that implements `Serialize + DeserializeOwned + Send + Sync + 'static`
///
/// # Examples
///
/// ```rust
/// use tako::middleware::jwt_auth::{JwtAuth, AnyVerifyKey};
/// use jwt_simple::prelude::*;
/// use serde::{Deserialize, Serialize};
/// use std::collections::HashMap;
/// use std::sync::Arc;
///
/// #[derive(Serialize, Deserialize, Clone)]
/// struct ApiClaims {
///     user_id: u32,
///     permissions: Vec<String>,
///     exp: u64,
/// }
///
/// // Single algorithm setup
/// let key = HS256Key::generate();
/// let mut keys = HashMap::new();
/// keys.insert("HS256", AnyVerifyKey::HS256(Arc::new(key)));
/// let auth = JwtAuth::<ApiClaims>::new(keys);
///
/// // Multi-algorithm setup for key rotation
/// let mut multi_keys = HashMap::new();
/// multi_keys.insert("HS256", AnyVerifyKey::HS256(Arc::new(HS256Key::generate())));
/// multi_keys.insert("RS256", AnyVerifyKey::RS256(Arc::new(
///     RS256PublicKey::from_pem("-----BEGIN PUBLIC KEY-----...").unwrap()
/// )));
/// let multi_auth = JwtAuth::<ApiClaims>::new(multi_keys);
/// ```
pub struct JwtAuth<T>
where
  T: DeserializeOwned + Send + Sync + 'static,
{
  /// Map of algorithm identifiers to verification keys.
  keys: Arc<HashMap<&'static str, AnyVerifyKey, BuildHasher>>,
  /// Phantom data for generic type association.
  _phantom: std::marker::PhantomData<T>,
}

impl<T> JwtAuth<T>
where
  T: DeserializeOwned + Send + Sync + 'static,
{
  /// Creates a new JWT authentication middleware with the specified verification keys.
  pub fn new(keys: HashMap<&'static str, AnyVerifyKey, BuildHasher>) -> Self {
    Self {
      keys: Arc::new(keys),
      _phantom: std::marker::PhantomData,
    }
  }
}

impl<T> IntoMiddleware for JwtAuth<T>
where
  T: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
{
  /// Converts the JWT authentication configuration into middleware.
  fn into_middleware(
    self,
  ) -> impl Fn(Request, Next) -> Pin<Box<dyn Future<Output = Response> + Send + 'static>>
  + Clone
  + Send
  + Sync
  + 'static {
    let keys = self.keys.clone();

    move |mut req: Request, next: Next| {
      let keys = keys.clone();

      Box::pin(async move {
        // Extract Bearer token from Authorization header
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

        // Decode token metadata to get algorithm
        let token_meta = match jwt_simple::token::Token::decode_metadata(token) {
          Ok(h) => h,
          Err(_) => {
            return (StatusCode::UNAUTHORIZED, "Cannot decode JWT header").into_response();
          }
        };

        // Find verification key for the token's algorithm
        let alg = &token_meta.algorithm();
        let verify_key = match keys.get(alg) {
          Some(k) => k,
          None => {
            return (
              StatusCode::UNAUTHORIZED,
              format!("Algorithm {alg} not allowed"),
            )
              .into_response();
          }
        };

        // Verify token and extract claims
        let claims = match verify_key.verify::<T>(token) {
          Ok(c) => c,
          Err(e) => {
            return (StatusCode::UNAUTHORIZED, format!("Invalid token: {e}")).into_response();
          }
        };

        // Inject claims into request extensions and continue
        req.extensions_mut().insert(claims);
        next.run(req).await.into_response()
      })
    }
  }
}
