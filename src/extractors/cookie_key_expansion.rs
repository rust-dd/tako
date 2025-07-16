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
///
/// Each context represents a different use case for derived keys, ensuring
/// that keys used for different purposes are cryptographically separated.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::cookie_key_expansion::KeyContext;
///
/// let signing_context = KeyContext::Signing;
/// let custom_context = KeyContext::Custom("user-tokens".to_string());
///
/// assert_eq!(signing_context.as_bytes(), b"cookie-signing");
/// assert_eq!(custom_context.as_bytes(), b"user-tokens");
/// ```
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
    ///
    /// Returns a consistent byte representation of the context that can be
    /// used as input to key derivation functions.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::KeyContext;
    ///
    /// assert_eq!(KeyContext::Signing.as_bytes(), b"cookie-signing");
    /// assert_eq!(KeyContext::Encryption.as_bytes(), b"cookie-encryption");
    /// assert_eq!(KeyContext::Csrf.as_bytes(), b"csrf-protection");
    /// assert_eq!(KeyContext::Session.as_bytes(), b"session-management");
    ///
    /// let custom = KeyContext::Custom("api-keys".to_string());
    /// assert_eq!(custom.as_bytes(), b"api-keys");
    /// ```
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
///
/// Contains the master key and application-specific parameters used for
/// deriving purpose-specific keys. The configuration ensures that derived
/// keys are unique to both the application and the specific use case.
///
/// # Examples
///
/// ```rust
/// use tako::extractors::cookie_key_expansion::KeyExpansionConfig;
/// use cookie::Key;
///
/// let master_key = Key::generate();
/// let config = KeyExpansionConfig::new(master_key, b"my-app-v1")
///     .with_key_length(32);
///
/// assert_eq!(config.key_length, 32);
/// ```
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
    ///
    /// # Arguments
    ///
    /// * `master_key` - The master key used as the source for all derived keys
    /// * `app_info` - Application-specific information used as salt in key derivation
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::KeyExpansionConfig;
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp-2024");
    ///
    /// assert_eq!(config.key_length, 32); // Default key length
    /// ```
    pub fn new(master_key: Key, app_info: impl Into<Vec<u8>>) -> Self {
        Self {
            master_key,
            app_info: app_info.into(),
            key_length: 32, // Default to 32 bytes (256 bits)
        }
    }

    /// Sets the key length for derived keys.
    ///
    /// # Arguments
    ///
    /// * `length` - The desired key length in bytes (must be between 16 and 64)
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::KeyExpansionConfig;
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp")
    ///     .with_key_length(24);
    ///
    /// assert_eq!(config.key_length, 24);
    /// ```
    pub fn with_key_length(mut self, length: usize) -> Self {
        self.key_length = length;
        self
    }
}

/// Cookie key expansion extractor for deriving purpose-specific keys.
///
/// This extractor provides methods for deriving cryptographic keys for different
/// purposes from a single master key. It integrates with the framework's request
/// system to automatically extract configuration from request extensions.
///
/// # Examples
///
/// ```rust,no_run
/// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyContext};
///
/// async fn handler(expansion: CookieKeyExpansion) -> Result<(), Box<dyn std::error::Error>> {
///     let signing_key = expansion.signing_key()?;
///     let encryption_key = expansion.encryption_key()?;
///
///     // Use keys for cookie operations
///     Ok(())
/// }
/// ```
pub struct CookieKeyExpansion {
    config: KeyExpansionConfig,
}

/// Error type for cookie key expansion operations.
///
/// Represents various failure modes that can occur during key derivation,
/// including missing configuration, invalid parameters, and derivation failures.
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
    ///
    /// All errors are mapped to appropriate HTTP status codes with descriptive
    /// error messages. Most errors result in `500 Internal Server Error` as they
    /// indicate server-side configuration issues.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::CookieKeyExpansionError;
    /// use tako::responder::Responder;
    /// use http::StatusCode;
    ///
    /// let error = CookieKeyExpansionError::MissingConfig;
    /// let response = error.into_response();
    /// assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    /// ```
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
                format!("Key derivation failed: {}", err),
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
    ///
    /// # Arguments
    ///
    /// * `config` - The key expansion configuration containing the master key and parameters
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    /// ```
    pub fn new(config: KeyExpansionConfig) -> Self {
        Self { config }
    }

    /// Derives a key for a specific context using simplified key derivation.
    ///
    /// # Arguments
    ///
    /// * `context` - The purpose for which the key will be used
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if:
    /// - The configured key length is invalid (not 16-64 bytes)
    /// - Key derivation fails
    /// - The derived key cannot be converted to a `cookie::Key`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig, KeyContext};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let signing_key = expansion.derive_key(KeyContext::Signing).unwrap();
    /// let encryption_key = expansion.derive_key(KeyContext::Encryption).unwrap();
    /// ```
    pub fn derive_key(&self, context: KeyContext) -> Result<Key, CookieKeyExpansionError> {
        self.derive_key_with_info(context, &[])
    }

    /// Derives a key for a specific context with additional info.
    ///
    /// Allows passing additional context-specific information that will be
    /// included in the key derivation process, further differentiating the
    /// derived key.
    ///
    /// # Arguments
    ///
    /// * `context` - The purpose for which the key will be used
    /// * `additional_info` - Extra information to include in key derivation
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if:
    /// - The configured key length is invalid (not 16-64 bytes)
    /// - Key derivation fails
    /// - The derived key cannot be converted to a `cookie::Key`
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig, KeyContext};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let user_key = expansion.derive_key_with_info(
    ///     KeyContext::Session,
    ///     b"user-12345"
    /// ).unwrap();
    /// ```
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
    ///
    /// Efficiently derives keys for multiple contexts in a single operation,
    /// returning a vector of context-key pairs.
    ///
    /// # Arguments
    ///
    /// * `contexts` - Slice of contexts for which to derive keys
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if any key derivation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig, KeyContext};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let contexts = vec![KeyContext::Signing, KeyContext::Encryption];
    /// let keys = expansion.derive_keys(&contexts).unwrap();
    ///
    /// assert_eq!(keys.len(), 2);
    /// ```
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
    ///
    /// Convenience method for deriving a key specifically for cookie signing operations.
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if key derivation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let signing_key = expansion.signing_key().unwrap();
    /// // Use signing_key for HMAC operations
    /// ```
    pub fn signing_key(&self) -> Result<Key, CookieKeyExpansionError> {
        self.derive_key(KeyContext::Signing)
    }

    /// Gets an encryption key for cookie operations.
    ///
    /// Convenience method for deriving a key specifically for cookie encryption operations.
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if key derivation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let encryption_key = expansion.encryption_key().unwrap();
    /// // Use encryption_key for AES operations
    /// ```
    pub fn encryption_key(&self) -> Result<Key, CookieKeyExpansionError> {
        self.derive_key(KeyContext::Encryption)
    }

    /// Gets a CSRF protection key.
    ///
    /// Convenience method for deriving a key specifically for CSRF token operations.
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if key derivation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let csrf_key = expansion.csrf_key().unwrap();
    /// // Use csrf_key for CSRF token generation
    /// ```
    pub fn csrf_key(&self) -> Result<Key, CookieKeyExpansionError> {
        self.derive_key(KeyContext::Csrf)
    }

    /// Gets a session management key.
    ///
    /// Convenience method for deriving a key specifically for session management operations.
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError` if key derivation fails.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let session_key = expansion.session_key().unwrap();
    /// // Use session_key for session token operations
    /// ```
    pub fn session_key(&self) -> Result<Key, CookieKeyExpansionError> {
        self.derive_key(KeyContext::Session)
    }

    /// Performs simplified key expansion.
    ///
    /// This is a simplified implementation for demonstration purposes. In production,
    /// use a proper HKDF implementation like the `hkdf` crate or similar cryptographic library.
    ///
    /// # Arguments
    ///
    /// * `info` - Additional information to include in the key derivation
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError::DerivationFailed` if the operation fails.
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
    ///
    /// Returns a reference to the internal configuration, allowing inspection
    /// of the key expansion parameters.
    ///
    /// # Examples
    ///
    /// ```rust
    /// use tako::extractors::cookie_key_expansion::{CookieKeyExpansion, KeyExpansionConfig};
    /// use cookie::Key;
    ///
    /// let master_key = Key::generate();
    /// let config = KeyExpansionConfig::new(master_key, "myapp");
    /// let expansion = CookieKeyExpansion::new(config);
    ///
    /// let config_ref = expansion.config();
    /// assert_eq!(config_ref.key_length, 32);
    /// ```
    pub fn config(&self) -> &KeyExpansionConfig {
        &self.config
    }

    /// Extracts key expansion from a request using config from extensions.
    ///
    /// # Arguments
    ///
    /// * `req` - The HTTP request containing configuration in extensions
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError::MissingConfig` if no configuration is found.
    fn extract_from_request(req: &Request) -> Result<Self, CookieKeyExpansionError> {
        let config = req
            .extensions()
            .get::<KeyExpansionConfig>()
            .ok_or(CookieKeyExpansionError::MissingConfig)?;

        Ok(Self::new(config.clone()))
    }

    /// Extracts key expansion from request parts using config from extensions.
    ///
    /// # Arguments
    ///
    /// * `parts` - The HTTP request parts containing configuration in extensions
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError::MissingConfig` if no configuration is found.
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

    /// Extracts `CookieKeyExpansion` from an HTTP request.
    ///
    /// The configuration must be placed in the request extensions before
    /// calling this extractor. This is typically done by middleware.
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError::MissingConfig` if no configuration
    /// is found in the request extensions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequest, cookie_key_expansion::CookieKeyExpansion};
    /// use tako::types::Request;
    ///
    /// async fn handler(mut req: Request) -> Result<(), Box<dyn std::error::Error>> {
    ///     let expansion = CookieKeyExpansion::from_request(&mut req).await?;
    ///     let signing_key = expansion.signing_key()?;
    ///     // Use the signing key...
    ///     Ok(())
    /// }
    /// ```
    fn from_request(
        req: &'a mut Request,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_request(req))
    }
}

impl<'a> FromRequestParts<'a> for CookieKeyExpansion {
    type Error = CookieKeyExpansionError;

    /// Extracts `CookieKeyExpansion` from HTTP request parts.
    ///
    /// The configuration must be placed in the request extensions before
    /// calling this extractor. This is typically done by middleware.
    ///
    /// # Errors
    ///
    /// Returns `CookieKeyExpansionError::MissingConfig` if no configuration
    /// is found in the request extensions.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use tako::extractors::{FromRequestParts, cookie_key_expansion::CookieKeyExpansion};
    /// use http::request::Parts;
    ///
    /// async fn handler(mut parts: Parts) -> Result<(), Box<dyn std::error::Error>> {
    ///     let expansion = CookieKeyExpansion::from_request_parts(&mut parts).await?;
    ///     let encryption_key = expansion.encryption_key()?;
    ///     // Use the encryption key...
    ///     Ok(())
    /// }
    /// ```
    fn from_request_parts(
        parts: &'a mut Parts,
    ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a
    {
        ready(Self::extract_from_parts(parts))
    }
}
