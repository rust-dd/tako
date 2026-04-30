//! Configuration loading from environment variables.
//!
//! Provides a `Config<T>` wrapper that can be loaded from environment variables
//! and injected as router state for access in handlers.
//!
//! # Examples
//!
//! ```rust
//! use tako::config::Config;
//! use serde::Deserialize;
//!
//! #[derive(Deserialize, Clone)]
//! struct AppConfig {
//!     database_url: String,
//!     port: u16,
//!     debug: bool,
//! }
//!
//! // Load from environment variables (DATABASE_URL, PORT, DEBUG)
//! // let config = Config::<AppConfig>::from_env().expect("missing config");
//! ```

use serde::de::DeserializeOwned;

/// A typed configuration wrapper loaded from environment variables.
///
/// `Config<T>` reads environment variables and deserializes them into a
/// strongly-typed struct. Variable names are matched by converting struct field names
/// to SCREAMING_SNAKE_CASE.
#[derive(Debug, Clone)]
pub struct Config<T: Clone>(pub T);

impl<T: DeserializeOwned + Clone> Config<T> {
  /// Loads configuration from environment variables.
  ///
  /// Field names are matched against environment variable names case-insensitively
  /// (`database_url` ↔ `DATABASE_URL`). Non-string fields (`u16`, `bool`, …) are
  /// parsed via the `envy` crate's per-field deserializers, so a typed
  /// `port: u16` reads `PORT=8080` natively without relying on JSON number
  /// coercion (which the previous serde_json-roundtrip implementation got wrong).
  pub fn from_env() -> Result<Self, ConfigError> {
    let config: T = envy::from_env::<T>().map_err(|e| ConfigError(e.to_string()))?;
    Ok(Config(config))
  }

  /// Loads configuration from environment variables that share a common prefix.
  ///
  /// Useful when several configs coexist in the process — set
  /// `MYAPP_DATABASE_URL`, `MYAPP_PORT`, … and call `Config::from_env_prefixed("MYAPP_")`.
  pub fn from_env_prefixed(prefix: &str) -> Result<Self, ConfigError> {
    let config: T = envy::prefixed(prefix)
      .from_env::<T>()
      .map_err(|e| ConfigError(e.to_string()))?;
    Ok(Config(config))
  }

  /// Creates a Config from an existing value.
  pub fn new(config: T) -> Self {
    Config(config)
  }

  /// Returns a reference to the inner config value.
  pub fn inner(&self) -> &T {
    &self.0
  }

  /// Consumes the wrapper and returns the inner value.
  pub fn into_inner(self) -> T {
    self.0
  }
}

/// Error type for configuration loading.
#[derive(Debug, Clone)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "configuration error: {}", self.0)
  }
}

impl std::error::Error for ConfigError {}
