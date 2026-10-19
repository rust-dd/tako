//! [`Config`], typed configuration loaded from environment variables.
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

/// `T` deserialized from environment variables named after its fields.
#[derive(Debug, Clone)]
pub struct Config<T: Clone>(pub T);

impl<T: DeserializeOwned + Clone> Config<T> {
  /// Matches variables to fields ignoring case (`DATABASE_URL` fills `database_url`) and
  /// parses typed fields such as `port: u16` directly.
  pub fn from_env() -> Result<Self, ConfigError> {
    let config: T = envy::from_env::<T>().map_err(|e| ConfigError(e.to_string()))?;
    Ok(Config(config))
  }

  /// [`Config::from_env`] over variables named `prefix` plus the field, e.g. `MYAPP_PORT`.
  pub fn from_env_prefixed(prefix: &str) -> Result<Self, ConfigError> {
    let config: T = envy::prefixed(prefix)
      .from_env::<T>()
      .map_err(|e| ConfigError(e.to_string()))?;
    Ok(Config(config))
  }

  pub fn new(config: T) -> Self {
    Config(config)
  }

  pub fn inner(&self) -> &T {
    &self.0
  }

  pub fn into_inner(self) -> T {
    self.0
  }
}

/// Why loading a [`Config`] failed, as reported by `envy`.
#[derive(Debug, Clone)]
pub struct ConfigError(pub String);

impl std::fmt::Display for ConfigError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    write!(f, "configuration error: {}", self.0)
  }
}

impl std::error::Error for ConfigError {}
