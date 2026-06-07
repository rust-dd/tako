//! Batteries-included multi-algorithm verifier built on `jwt-simple`.

use std::collections::HashMap;
use std::sync::Arc;

use ::jwt_simple::prelude::*;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tako_rs_core::types::BuildHasher;

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
  keys_by_kid: Arc<parking_lot::RwLock<HashMap<String, AnyVerifyKey>>>,
  constraints: Arc<super::VerifyConstraints>,
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
      keys_by_kid: Arc::new(parking_lot::RwLock::new(HashMap::new())),
      constraints: Arc::new(super::VerifyConstraints::default()),
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
    self.constraints = Arc::new(c);
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

    let mut opts = VerificationOptions {
      time_tolerance: Some(::jwt_simple::prelude::Duration::from_secs(
        self.constraints.leeway_secs,
      )),
      ..Default::default()
    };
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

    key
      .verify_token::<C>(token, opts)
      .map_err(|e| e.to_string())
  }

  fn validate_constraints(
    &self,
    claims: &Self::Claims,
    constraints: &super::VerifyConstraints,
  ) -> Result<(), super::ConstraintsNotSupported> {
    if let Some(expected) = &constraints.issuer
      && claims.issuer.as_deref() != Some(expected.as_str())
    {
      return Err(super::ConstraintsNotSupported {
        reason: "issuer mismatch",
      });
    }
    if let Some(expected) = &constraints.audience {
      let mut allowed = std::collections::HashSet::new();
      allowed.insert(expected.clone());
      match &claims.audiences {
        Some(a) if a.contains(&allowed) => {}
        _ => {
          return Err(super::ConstraintsNotSupported {
            reason: "audience mismatch",
          });
        }
      }
    }
    // `leeway_secs` is applied to exp/nbf by the underlying verify() call
    // when this verifier's internal `constraints.leeway_secs` is set; the
    // middleware-level field is informational only here. If both are set
    // and disagree, the verifier-level leeway wins for exp/nbf and the
    // middleware-level leeway is ignored.
    Ok(())
  }
}
