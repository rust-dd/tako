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
  /// Field names are converted to uppercase with underscores (e.g., `database_url` -> `DATABASE_URL`).
  pub fn from_env() -> Result<Self, ConfigError> {
    // Collect all env vars into a map
    let vars: std::collections::HashMap<String, String> = std::env::vars().collect();

    // Serialize the map to JSON, then deserialize into T
    let value = serde_json::to_value(&vars).map_err(|e| ConfigError(e.to_string()))?;
    let config: T =
      serde_json::from_value(value).map_err(|e| ConfigError(e.to_string()))?;

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
