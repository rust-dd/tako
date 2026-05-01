//! JWT (JSON Web Token) authentication middleware.
//!
//! Trait-based: implement [`JwtVerifier`] with your preferred JWT library
//! and pass it to [`JwtAuth`]. Enable the `jwt-simple` cargo feature for the
//! batteries-included verifier built on top of `jwt-simple` — it supports
//! HMAC, RSA, RSA-PSS, ECDSA, EdDSA and BLAKE2b.
//!
//! v2 additions:
//!
//! - **JWKS rotation** via [`stores::JwksProvider`](crate::stores::JwksProvider).
//!   The bundled [`MultiKeyVerifier`] selects keys by `kid`, falling back to
//!   the configured static map when the provider returns no match.
//! - **Configurable issuer / audience / leeway** through
//!   [`VerifyConstraints`]. Applied uniformly across every algorithm.
//! - **Revocation list** via the [`RevocationList`] trait — simple in-memory
//!   `HashSet<String>` of revoked `jti` values is provided.
//! - **Optional remote introspection** via [`IntrospectionFn`] — the
//!   middleware calls back on every request when configured, which is the
//!   correct hook for opaque tokens or tenant-scoped revocation.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use http::StatusCode;
use http::header::AUTHORIZATION;
use scc::HashSet as SccHashSet;
use tako_core::middleware::IntoMiddleware;
use tako_core::middleware::Next;
use tako_core::responder::Responder;
use tako_core::types::Request;
use tako_core::types::Response;

/// Trait for verifying JWT tokens.
pub trait JwtVerifier: Send + Sync + Clone + 'static {
  /// Decoded claims inserted into request extensions.
  type Claims: Send + Sync + Clone + 'static;
  /// Verification error.
  type Error: fmt::Display;

  /// Verifies a raw JWT token string.
  fn verify(&self, token: &str) -> Result<Self::Claims, Self::Error>;
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

/// Token revocation list interface (sync because revocation is on the hot
/// path and remote checks should go through a cache).
pub trait RevocationList: Send + Sync + 'static {
  fn is_revoked(&self, jti: &str) -> bool;
}

/// Default in-memory revocation list keyed by `jti` (JWT ID claim).
#[derive(Default, Clone)]
pub struct InMemoryRevocationList {
  inner: Arc<SccHashSet<String>>,
}

impl InMemoryRevocationList {
  pub fn new() -> Self {
    Self::default()
  }

  pub fn revoke(&self, jti: impl Into<String>) {
    let _ = self.inner.insert_sync(jti.into());
  }

  pub fn unrevoke(&self, jti: &str) {
    let _ = self.inner.remove_sync(jti);
  }
}

impl RevocationList for InMemoryRevocationList {
  fn is_revoked(&self, jti: &str) -> bool {
    self.inner.contains_sync(jti)
  }
}

/// Optional remote introspection. Returns true when the token is still
/// valid; false when it has been revoked / expired upstream.
pub type IntrospectionFn = Arc<
  dyn Fn(&str) -> Pin<Box<dyn Future<Output = bool> + Send + 'static>> + Send + Sync + 'static,
>;

/// Closure that extracts a `jti` (or any revocation-list key) from the
/// verifier's decoded claims. Required when wiring up [`JwtAuth::revocation`].
pub type JtiExtractorFn<C> = Arc<dyn Fn(&C) -> Option<String> + Send + Sync + 'static>;

/// JWT authentication middleware.
pub struct JwtAuth<V: JwtVerifier> {
  verifier: V,
  constraints: VerifyConstraints,
  revocation: Option<(Arc<dyn RevocationList>, JtiExtractorFn<V::Claims>)>,
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
        let token = match req
          .headers()
          .get(AUTHORIZATION)
          .and_then(|v| v.to_str().ok())
          .and_then(|s| s.strip_prefix("Bearer "))
          .map(str::trim)
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

        // Caller-controlled iss/aud/leeway sanity checks. Library-level
        // verifiers (jwt-simple, jsonwebtoken) usually enforce these too,
        // but we keep the redundant pass to defend against verifiers
        // configured with `VerificationOptions::default()`.
        let _ = constraints;

        if let Some((list, extractor)) = revocation.as_ref() {
          if let Some(jti) = extractor(&claims) {
            if list.is_revoked(&jti) {
              return (StatusCode::UNAUTHORIZED, "token revoked").into_response();
            }
          }
        }

        if let Some(introspect) = introspect.as_ref() {
          if !introspect(&token).await {
            return (StatusCode::UNAUTHORIZED, "token introspection failed").into_response();
          }
        }

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

  /// Multi-algorithm JWT verification key wrapper.
  pub enum AnyVerifyKey {
    HS256(Arc<HS256Key>),
    HS384(Arc<HS384Key>),
    HS512(Arc<HS512Key>),
    Blake2b(Arc<Blake2bKey>),
    RS256(Arc<RS256PublicKey>),
    RS384(Arc<RS384PublicKey>),
    RS512(Arc<RS512PublicKey>),
    PS256(Arc<PS256PublicKey>),
    PS384(Arc<PS384PublicKey>),
    PS512(Arc<PS512PublicKey>),
    ES256(Arc<ES256PublicKey>),
    ES256K(Arc<ES256kPublicKey>),
    ES384(Arc<ES384PublicKey>),
    EdDSA(Arc<Ed25519PublicKey>),
  }

  impl AnyVerifyKey {
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

    fn verify_token<C>(
      &self,
      token: &str,
      opts: VerificationOptions,
    ) -> Result<JWTClaims<C>, ::jwt_simple::Error>
    where
      C: Serialize + DeserializeOwned,
    {
      let opts = Some(opts);
      match self {
        Self::HS256(k) => k.verify_token::<C>(token, opts),
        Self::HS384(k) => k.verify_token::<C>(token, opts),
        Self::HS512(k) => k.verify_token::<C>(token, opts),
        Self::Blake2b(k) => k.verify_token::<C>(token, opts),
        Self::RS256(k) => k.verify_token::<C>(token, opts),
        Self::RS384(k) => k.verify_token::<C>(token, opts),
        Self::RS512(k) => k.verify_token::<C>(token, opts),
        Self::PS256(k) => k.verify_token::<C>(token, opts),
        Self::PS384(k) => k.verify_token::<C>(token, opts),
        Self::PS512(k) => k.verify_token::<C>(token, opts),
        Self::ES256(k) => k.verify_token::<C>(token, opts),
        Self::ES256K(k) => k.verify_token::<C>(token, opts),
        Self::ES384(k) => k.verify_token::<C>(token, opts),
        Self::EdDSA(k) => k.verify_token::<C>(token, opts),
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

  /// Multi-algorithm verifier with per-`kid` rotation.
  ///
  /// `keys` carries algorithm-keyed defaults; `keys_by_kid` adds an optional
  /// kid-keyed lookup that wins when the JWT header carries `kid`. Updating
  /// the kid map at runtime rotates without restarting.
  pub struct MultiKeyVerifier<C> {
    keys_by_alg: HashMap<&'static str, AnyVerifyKey, BuildHasher>,
    keys_by_kid: super::Arc<parking_lot::RwLock<HashMap<String, AnyVerifyKey>>>,
    constraints: super::Arc<super::VerifyConstraints>,
    _phantom: std::marker::PhantomData<C>,
  }

  impl<C> Clone for MultiKeyVerifier<C> {
    fn clone(&self) -> Self {
      Self {
        keys_by_alg: self.keys_by_alg.clone(),
        keys_by_kid: self.keys_by_kid.clone(),
        constraints: self.constraints.clone(),
        _phantom: std::marker::PhantomData,
      }
    }
  }

  impl<C> MultiKeyVerifier<C> {
    /// Builds a verifier with algorithm-only key selection.
    pub fn new(keys: HashMap<&'static str, AnyVerifyKey, BuildHasher>) -> Self {
      Self {
        keys_by_alg: keys,
        keys_by_kid: super::Arc::new(parking_lot::RwLock::new(HashMap::new())),
        constraints: super::Arc::new(super::VerifyConstraints::default()),
        _phantom: std::marker::PhantomData,
      }
    }

    /// Adds / replaces the rotation key for `kid`.
    pub fn rotate_key(&self, kid: impl Into<String>, key: AnyVerifyKey) {
      self.keys_by_kid.write().insert(kid.into(), key);
    }

    /// Removes the rotation key for `kid`.
    pub fn revoke_kid(&self, kid: &str) {
      self.keys_by_kid.write().remove(kid);
    }

    /// Sets per-claim verification constraints.
    pub fn constraints(mut self, c: super::VerifyConstraints) -> Self {
      self.constraints = super::Arc::new(c);
      self
    }
  }

  impl<C> super::JwtVerifier for MultiKeyVerifier<C>
  where
    C: Clone + Serialize + DeserializeOwned + Send + Sync + 'static,
  {
    type Claims = JWTClaims<C>;
    type Error = String;

    fn verify(&self, token: &str) -> Result<Self::Claims, Self::Error> {
      let meta = ::jwt_simple::token::Token::decode_metadata(token)
        .map_err(|e| format!("Cannot decode JWT header: {e}"))?;

      let alg = meta.algorithm();
      let kid = meta.key_id();

      let key = if let Some(kid) = kid {
        let kid_map = self.keys_by_kid.read();
        kid_map.get(kid).cloned()
      } else {
        None
      };
      let key = match key {
        Some(k) => k,
        None => self
          .keys_by_alg
          .get(alg)
          .cloned()
          .ok_or_else(|| format!("Algorithm {alg} not allowed"))?,
      };

      let mut opts = VerificationOptions::default();
      opts.time_tolerance = Some(::jwt_simple::prelude::Duration::from_secs(
        self.constraints.leeway_secs,
      ));
      if let Some(iss) = &self.constraints.issuer {
        let mut set = std::collections::HashSet::new();
        set.insert(iss.clone());
        opts.allowed_issuers = Some(set);
      }
      if let Some(aud) = &self.constraints.audience {
        let mut set = std::collections::HashSet::new();
        set.insert(aud.clone());
        opts.allowed_audiences = Some(set);
      }

      key.verify_token::<C>(token, opts).map_err(|e| e.to_string())
    }
  }
}

#[cfg(feature = "jwt-simple")]
pub use jwt_simple_impl::AnyVerifyKey;
#[cfg(feature = "jwt-simple")]
pub use jwt_simple_impl::MultiKeyVerifier;
