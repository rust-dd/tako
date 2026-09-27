use base64::Engine as _;

use super::JwtVerifier;
use crate::stores::JwksProvider;
use crate::stores::StoreResult;

#[derive(serde::Deserialize)]
struct Metadata {
  kid: Option<String>,
}

pub(crate) async fn verify<V: JwtVerifier>(
  verifier: &V,
  token: &str,
  provider: Option<&dyn JwksProvider>,
) -> StoreResult<Option<V::Claims>> {
  if let Some(provider) = provider {
    let Some(header) = token.split('.').next() else {
      return Ok(None);
    };
    let Ok(bytes) = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(header) else {
      return Ok(None);
    };
    let Ok(metadata) = serde_json::from_slice::<Metadata>(&bytes) else {
      return Ok(None);
    };
    if let Some(kid) = metadata.kid {
      if kid.is_empty() {
        return Ok(None);
      }
      let keys = provider.keys_for(&kid).await?;
      if !keys.is_empty() {
        for key in keys {
          if let Some(Ok(claims)) = verifier.verify_with_key(token, &key) {
            return Ok(Some(claims));
          }
        }
        return Ok(None);
      }
    }
  }
  match verifier.verify(token) {
    Ok(claims) => Ok(Some(claims)),
    Err(error) => {
      tracing::debug!(%error, "JWT signature verification failed");
      Ok(None)
    }
  }
}
