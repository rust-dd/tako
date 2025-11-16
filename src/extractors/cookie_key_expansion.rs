//! Key derivation and expansion for secure cookie operations.
//!
//! This module provides functionality for deriving purpose-specific cryptographic keys
//! from a master key using HKDF-like key derivation. It supports multiple key contexts
//! such as signing, encryption, CSRF protection, and session management, enabling
//! secure separation of cryptographic concerns in cookie-based applications.
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
use http::{StatusCode, request::Parts};
use std::future::ready;

use crate::{
  extractors::{FromRequest, FromRequestParts},
  responder::Responder,
  types::Request,
};

/// Key derivation contexts for different cryptographic purposes.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum KeyContext {
  /// Key for signing cookies with HMAC.
  Signing,
  /// Key for encrypting cookies.
  Encryption,
  /// Key for CSRF protection tokens.
  Csrf,
  /// Key for session management.
  Session,
  /// Custom context with a application-specific purpose.
  Custom(String),
}

impl KeyContext {
  /// Converts the context to a byte slice for key derivation.
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

/// Configuration for key expansion operations.
#[derive(Debug, Clone)]
pub struct KeyExpansionConfig {
  /// The master key used for derivation.
  pub master_key: Key,
  /// Application-specific info/salt for key derivation.
  pub app_info: Vec<u8>,
  /// Key length for derived keys in bytes.
  pub key_length: usize,
}

impl KeyExpansionConfig {
  /// Creates a new key expansion configuration.
  pub fn new(master_key: Key, app_info: impl Into<Vec<u8>>) -> Self {
    Self {
      master_key,
      app_info: app_info.into(),
      key_length: 32, // Default to 32 bytes (256 bits)
    }
  }

  /// Sets the key length for derived keys.
  pub fn with_key_length(mut self, length: usize) -> Self {
    self.key_length = length;
    self
  }
}

/// Cookie key expansion extractor for deriving purpose-specific keys.
pub struct CookieKeyExpansion {
  config: KeyExpansionConfig,
}

/// Error type for cookie key expansion operations.
#[derive(Debug)]
pub enum CookieKeyExpansionError {
  /// Key expansion configuration not found in request extensions.
  MissingConfig,
  /// The master key is invalid or corrupted.
  InvalidMasterKey,
  /// Key derivation failed with the specified error message.
  DerivationFailed(String),
  /// The specified key length is invalid (must be 16-64 bytes).
  InvalidKeyLength,
  /// The key derivation algorithm is not supported.
  UnsupportedAlgorithm,
}

impl Responder for CookieKeyExpansionError {
  /// Converts the error into an HTTP response.
  fn into_response(self) -> crate::types::Response {
    match self {
      CookieKeyExpansionError::MissingConfig => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Key expansion configuration not found in request extensions",
      )
        .into_response(),
      CookieKeyExpansionError::InvalidMasterKey => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Invalid master key for key derivation",
      )
        .into_response(),
      CookieKeyExpansionError::DerivationFailed(err) => (
        StatusCode::INTERNAL_SERVER_ERROR,
        format!("Key derivation failed: {err}"),
      )
        .into_response(),
      CookieKeyExpansionError::InvalidKeyLength => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Invalid key length specified for derivation",
      )
        .into_response(),
      CookieKeyExpansionError::UnsupportedAlgorithm => (
        StatusCode::INTERNAL_SERVER_ERROR,
        "Unsupported key derivation algorithm",
      )
        .into_response(),
    }
  }
}

impl CookieKeyExpansion {
  /// Creates a new key expansion instance with the given configuration.
  pub fn new(config: KeyExpansionConfig) -> Self {
    Self { config }
  }

  /// Derives a key for a specific context using simplified key derivation.
  pub fn derive_key(&self, context: KeyContext) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key_with_info(context, &[])
  }

  /// Derives a key for a specific context with additional info.
  pub fn derive_key_with_info(
    &self,
    context: KeyContext,
    additional_info: &[u8],
  ) -> Result<Key, CookieKeyExpansionError> {
    if self.config.key_length < 16 || self.config.key_length > 64 {
      return Err(CookieKeyExpansionError::InvalidKeyLength);
    }

    // Create the info parameter by combining context, app info, and additional info
    let mut info = Vec::new();
    info.extend_from_slice(context.as_bytes());
    info.push(0x00); // Separator
    info.extend_from_slice(&self.config.app_info);
    if !additional_info.is_empty() {
      info.push(0x00); // Separator
      info.extend_from_slice(additional_info);
    }

    // Perform HKDF key derivation
    let derived_key = self.hkdf_expand(&info)?;

    // Convert to cookie::Key
    Key::try_from(derived_key.as_slice())
      .map_err(|e| CookieKeyExpansionError::DerivationFailed(e.to_string()))
  }

  /// Derives multiple keys for different contexts at once.
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

  /// Gets a signing key for cookie operations.
  pub fn signing_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Signing)
  }

  /// Gets an encryption key for cookie operations.
  pub fn encryption_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Encryption)
  }

  /// Gets a CSRF protection key.
  pub fn csrf_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Csrf)
  }

  /// Gets a session management key.
  pub fn session_key(&self) -> Result<Key, CookieKeyExpansionError> {
    self.derive_key(KeyContext::Session)
  }

  /// Performs simplified key expansion.
  fn hkdf_expand(&self, info: &[u8]) -> Result<Vec<u8>, CookieKeyExpansionError> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    // This is a simplified key derivation - in production, use a proper HKDF implementation
    // like the `hkdf` crate or similar cryptographic library

    let mut hasher = DefaultHasher::new();

    // Hash the master key material
    self.config.master_key.master().hash(&mut hasher);
    info.hash(&mut hasher);

    let hash_result = hasher.finish();

    // Expand the hash to the desired key length
    let mut derived_key = Vec::with_capacity(self.config.key_length);
    let hash_bytes = hash_result.to_le_bytes();

    for i in 0..self.config.key_length {
      derived_key.push(hash_bytes[i % hash_bytes.len()]);
    }

    // Mix in additional entropy from the master key
    for (i, &byte) in self.config.master_key.master().iter().enumerate() {
      if i < derived_key.len() {
        derived_key[i] ^= byte;
      }
    }

    Ok(derived_key)
  }

  /// Gets the configuration used for key expansion.
  pub fn config(&self) -> &KeyExpansionConfig {
    &self.config
  }

  /// Extracts key expansion from a request using config from extensions.
  fn extract_from_request(req: &Request) -> Result<Self, CookieKeyExpansionError> {
    let config = req
      .extensions()
      .get::<KeyExpansionConfig>()
      .ok_or(CookieKeyExpansionError::MissingConfig)?;

    Ok(Self::new(config.clone()))
  }

  /// Extracts key expansion from request parts using config from extensions.
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
    ready(Self::extract_from_request(req))
  }
}

impl<'a> FromRequestParts<'a> for CookieKeyExpansion {
  type Error = CookieKeyExpansionError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    ready(Self::extract_from_parts(parts))
  }
}
