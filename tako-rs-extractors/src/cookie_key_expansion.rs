//! Purpose-specific cookie keys derived from one master key with HKDF-SHA256 (RFC 5869).
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig, KeyContext};
//! use cookie::Key;
//!
//! // Create a master key and configuration
//! let master_key = Key::generate();
//! let config = KeyExpansionConfig::new(master_key, b"my-app");
//! let expansion = CookieKeyExpansion::new(config);
//!
//! // Derive keys for different purposes
//! let signing_key = expansion.signing_key().unwrap();
//! let encryption_key = expansion.encryption_key().unwrap();
//! ```

use cookie::Key;
use hkdf::Hkdf;
use http::StatusCode;
use http::request::Parts;
use sha2::Sha256;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// The purpose a derived key is bound to.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyContext {
  Signing,
  Encryption,
  Csrf,
  Session,
  /// Bytes equal to a built-in context's, such as `cookie-signing`, derive that context's key.
  Custom(String),
}

impl KeyContext {
  /// The HKDF `info` bytes.
  pub fn as_bytes(&self) -> &[u8] {
    match self {
      KeyContext::Signing => b"cookie-signing",
      KeyContext::Encryption => b"cookie-encryption",
      KeyContext::Csrf => b"csrf-protection",
      KeyContext::Session => b"session-management",
      KeyContext::Custom(purpose) => purpose.as_bytes(),
    }
  }
}

/// HKDF inputs: the master key is the input keying material and `app_info` the salt.
#[derive(Debug, Clone)]
pub struct KeyExpansionConfig {
  pub master_key: Key,
  pub app_info: Vec<u8>,
}

impl KeyExpansionConfig {
  pub fn new(master_key: Key, app_info: impl Into<Vec<u8>>) -> Self {
    Self {
      master_key,
      app_info: app_info.into(),
    }
  }
}

/// Derives keys from a [`KeyExpansionConfig`]; as an extractor it reads the config from the
/// request extensions.
pub struct CookieKeyExpansion {
  config: KeyExpansionConfig,
}

/// Key derivation failure; as a rejection every variant responds 500.
#[derive(Debug)]
pub enum CookieKeyExpansionError {
  /// No `KeyExpansionConfig` in the request extensions.
  MissingConfig,
  DerivationFailed(String),
}

impl Responder for CookieKeyExpansionError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      CookieKeyExpansionError::MissingConfig => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Key expansion configuration not found in request extensions",
      )
        .into_response(),
      CookieKeyExpansionError::DerivationFailed(err) => (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Key derivation failed: {err}"),
      )
        .into_response(),
    }
  }
}

impl CookieKeyExpansion {
  pub fn new(config: KeyExpansionConfig) -> Self {
    Self { config }
  }

  /// [`Self::derive_key_with_info`] without additional info.
  pub fn derive_key(&self, context: KeyContext) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key_with_info(context, &[])
  }

  /// The HKDF info is the context's bytes, then, when `additional_info` is non-empty, a zero
  /// byte and `additional_info`.
  pub fn derive_key_with_info(
    &self,
    context: KeyContext,
    additional_info: &[u8],
  ) -> Result<Key, CookieKeyExpansionError> {
    let mut info = Vec::with_capacity(context.as_bytes().len() + 1 + additional_info.len());
    info.extend_from_slice(context.as_bytes());
    if !additional_info.is_empty() {
      info.push(0x00);
      info.extend_from_slice(additional_info);
    }

    let derived_key = self.hkdf_expand(&info)?;

    Key::try_from(derived_key.as_slice())
      .map_err(|e| CookieKeyExpansionError::DerivationFailed(e.to_string()))
  }

  pub fn derive_keys(
    &self,
    contexts: &[KeyContext],
  ) -> Result<Vec<(KeyContext, Key)>, CookieKeyExpansionError> {
    contexts
      .iter()
      .map(|context| {
        let key = self.derive_key(context.clone())?;
        Ok((context.clone(), key))
      })
      .collect()
  }

  pub fn signing_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Signing)
  }

  pub fn encryption_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Encryption)
  }

  pub fn csrf_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Csrf)
  }

  pub fn session_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Session)
  }

  fn hkdf_expand(&self, info: &[u8]) -> Result<Vec<u8>, CookieKeyExpansionError> {
    let hk = Hkdf::<Sha256>::new(Some(&self.config.app_info), self.config.master_key.master());
    // `cookie::Key` takes exactly 64 bytes.
    let mut okm = vec![0u8; 64];
    hk.expand(info, &mut okm)
      .map_err(|e| CookieKeyExpansionError::DerivationFailed(e.to_string()))?;
    Ok(okm)
  }

  pub fn config(&self) -> &KeyExpansionConfig {
    &self.config
  }

  fn extract_from_request(req: &Request) -> Result<Self, CookieKeyExpansionError> {
    let config = req
      .extensions()
      .get::<KeyExpansionConfig>()
      .ok_or(CookieKeyExpansionError::MissingConfig)?;

    Ok(Self::new(config.clone()))
  }

  fn extract_from_parts(parts: &Parts) -> Result<Self, CookieKeyExpansionError> {
    let config = parts
      .extensions
      .get::<KeyExpansionConfig>()
      .ok_or(CookieKeyExpansionError::MissingConfig)?;

    Ok(Self::new(config.clone()))
  }
}

impl<'a> FromRequest<'a> for CookieKeyExpansion {
  type Error = CookieKeyExpansionError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_request(req))
  }
}

impl<'a> FromRequestParts<'a> for CookieKeyExpansion {
  type Error = CookieKeyExpansionError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_from_parts(parts))
  }
}
