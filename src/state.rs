//! Global application state management and dependency injection.
//!
//! This module provides a thread-safe global state store that allows sharing data across
//! different parts of the application. State values are stored by string keys and can be
//! retrieved with type safety. The state system uses `Arc` for shared ownership and
//! `Any` trait for type erasure, enabling storage of arbitrary types that implement
//! `Send + Sync + 'static`.
//!
//! # Examples
//!
//! ```rust
//! use tako::state::{set_state, get_state};
//!
//! #[derive(Clone, Debug, PartialEq)]
//! struct DatabaseConfig {
//!     url: String,
//!     max_connections: u32,
//! }
//!
//! // Store configuration in global state
//! let config = DatabaseConfig {
//!     url: "postgresql://localhost/mydb".to_string(),
//!     max_connections: 10,
//! };
//! set_state("db_config", config.clone());
//!
//! // Retrieve configuration from global state
//! let retrieved: Option<std::sync::Arc<DatabaseConfig>> = get_state("db_config");
//! assert_eq!(retrieved.as_ref().map(|c| &**c), Some(&config));
//! ```

use std::{
    any::{Any, TypeId},
    sync::Arc,
};

use dashmap::DashMap;
use once_cell::sync::Lazy;

/// Global state storage using thread-safe concurrent hash map.
///
/// This static variable holds the global application state, allowing values to be
/// shared across different parts of the application. Values are stored as type-erased
/// `Arc<dyn Any + Send + Sync>` to enable storage of arbitrary types while maintaining
/// thread safety.
pub(crate) static GLOBAL_STATE: Lazy<DashMap<TypeId, Arc<dyn Any + Send + Sync>>> =
    Lazy::new(|| DashMap::new());

/// Stores a value in the global state under the specified key.
///
/// The value is wrapped in an `Arc` and stored with type erasure, allowing it to be
/// retrieved later with the correct type. If a value already exists for the given key,
/// it will be replaced with the new value.
///
/// # Examples
///
/// ```rust
/// use tako::state::set_state;
///
/// // Store a string value
/// set_state("app_name", "My Application".to_string());
///
/// // Store a numeric value
/// set_state("max_users", 1000u32);
///
/// // Store a custom struct
/// #[derive(Clone)]
/// struct Config {
///     debug: bool,
///     timeout: u64,
/// }
///
/// let config = Config { debug: true, timeout: 30 };
/// set_state("config", config);
/// ```
pub fn set_state<T: Send + Sync + 'static>(value: T) {
    GLOBAL_STATE.insert(TypeId::of::<T>(), Arc::new(value));
}

/// Retrieves a value from the global state by its key.
///
/// Attempts to find and downcast the stored value to the requested type. Returns
/// `Some(Arc<T>)` if the key exists and the value can be downcast to type `T`,
/// or `None` if the key doesn't exist or the type doesn't match.
///
/// # Examples
///
/// ```rust
/// use tako::state::{set_state, get_state};
/// use std::sync::Arc;
///
/// // Store and retrieve a string
/// set_state("message", "Hello, World!".to_string());
/// let message: Option<Arc<String>> = get_state("message");
/// assert_eq!(message.as_ref().map(|s| s.as_str()), Some("Hello, World!"));
///
/// // Attempt to retrieve with wrong type returns None
/// let wrong_type: Option<Arc<u32>> = get_state("message");
/// assert!(wrong_type.is_none());
///
/// // Retrieve non-existent key returns None
/// let missing: Option<Arc<String>> = get_state("nonexistent");
/// assert!(missing.is_none());
/// ```
pub fn get_state<T: Send + Sync + 'static>() -> Option<Arc<T>> {
    GLOBAL_STATE
        .get(&TypeId::of::<T>())
        .map(|v| v.clone())
        .and_then(|v| v.downcast::<T>().ok())
}
