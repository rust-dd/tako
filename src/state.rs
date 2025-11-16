//! Global application state management and dependency injection (type-based).
//!
//! This module provides a thread-safe global state store for sharing data across your
//! application. Values are stored by their concrete type (not by string keys) and can be
//! retrieved with type safety. The store uses `Arc` and `Any` for type erasure and shared
//! ownership of values that implement `Send + Sync + 'static`.
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
//! // Store configuration by type
//! let config = DatabaseConfig {
//!     url: "postgresql://localhost/mydb".to_string(),
//!     max_connections: 10,
//! };
//! set_state(config.clone());
//!
//! // Retrieve configuration by type
//! let retrieved: Option<std::sync::Arc<DatabaseConfig>> = get_state::<DatabaseConfig>();
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

/// Stores a value in the global state, keyed by its concrete type `T`.
///
/// The value is wrapped in an `Arc` and can later be retrieved with [`get_state::<T>`].
/// Storing again for the same type replaces the previous value.
///
/// # Examples
///
/// ```rust
/// use tako::state::set_state;
///
/// // Store a string by type
/// set_state::<String>("My Application".to_string());
///
/// // Store a numeric value
/// set_state::<u32>(1000);
///
/// // Store a custom struct
/// #[derive(Clone)]
/// struct Config { debug: bool, timeout: u64 }
///
/// let config = Config { debug: true, timeout: 30 };
/// set_state(config);
/// ```
pub fn set_state<T: Send + Sync + 'static>(value: T) {
  GLOBAL_STATE.insert(TypeId::of::<T>(), Arc::new(value));
}

/// Retrieves a value from the global state by its concrete type `T`.
///
/// Returns `Some(Arc<T>)` if a value was previously stored for `T`, or `None` otherwise.
///
/// # Examples
///
/// ```rust
/// use tako::state::{set_state, get_state};
/// use std::sync::Arc;
///
/// // Store and retrieve a string
/// set_state::<String>("Hello, World!".to_string());
/// let message: Option<Arc<String>> = get_state::<String>();
/// assert_eq!(message.as_ref().map(|s| s.as_str()), Some("Hello, World!"));
///
/// // Attempt to retrieve with wrong type returns None
/// let wrong_type: Option<Arc<u32>> = get_state::<u32>();
/// assert!(wrong_type.is_none());
///
/// // Non-existent type returns None
/// let missing: Option<Arc<Vec<u8>>> = get_state::<Vec<u8>>();
/// assert!(missing.is_none());
/// ```
pub fn get_state<T: Send + Sync + 'static>() -> Option<Arc<T>> {
  GLOBAL_STATE
    .get(&TypeId::of::<T>())
    .map(|v| v.clone())
    .and_then(|v| v.downcast::<T>().ok())
}
