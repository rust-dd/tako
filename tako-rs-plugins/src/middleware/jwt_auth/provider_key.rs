use std::sync::Arc;

use jwt_simple::prelude::*;

use super::AnyVerifyKey;

pub(crate) fn decode(algorithm: &str, bytes: &[u8]) -> Result<AnyVerifyKey, String> {
  macro_rules! public_key {
    ($type:ty, $variant:ident) => {
      AnyVerifyKey::$variant(Arc::new(
        <$type>::from_der(bytes).map_err(|error| error.to_string())?,
      ))
    };
  }
  Ok(match algorithm {
    "HS256" => AnyVerifyKey::HS256(Arc::new(HS256Key::from_bytes(bytes))),
    "HS384" => AnyVerifyKey::HS384(Arc::new(HS384Key::from_bytes(bytes))),
    "HS512" => AnyVerifyKey::HS512(Arc::new(HS512Key::from_bytes(bytes))),
    "BLAKE2B" => AnyVerifyKey::Blake2b(Arc::new(Blake2bKey::from_bytes(bytes))),
    "RS256" => public_key!(RS256PublicKey, RS256),
    "RS384" => public_key!(RS384PublicKey, RS384),
    "RS512" => public_key!(RS512PublicKey, RS512),
    "PS256" => public_key!(PS256PublicKey, PS256),
    "PS384" => public_key!(PS384PublicKey, PS384),
    "PS512" => public_key!(PS512PublicKey, PS512),
    "ES256" => public_key!(ES256PublicKey, ES256),
    "ES256K" => public_key!(ES256kPublicKey, ES256K),
    "ES384" => public_key!(ES384PublicKey, ES384),
    "EdDSA" => public_key!(Ed25519PublicKey, EdDSA),
    _ => return Err("unsupported key algorithm".into()),
  })
}
