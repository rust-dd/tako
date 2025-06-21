use std::{any::Any, sync::Arc};

use dashmap::DashMap;
use once_cell::sync::Lazy;

pub(crate) static GLOBAL_STATE: Lazy<DashMap<String, Arc<dyn Any + Send + Sync>>> =
    Lazy::new(|| DashMap::new());

/// Stores a value in the global state under the specified key.
///
/// # Arguments
///
/// * `key` - A string slice that holds the key under which the value will be stored.
/// * `value` - The value to store in the global state. It must implement `Send`, `Sync`, and `'static`.
pub fn set_state<T: Send + Sync + 'static>(key: &str, value: T) {
    GLOBAL_STATE.insert(key.to_string(), Arc::new(value));
}

/// Retrieves a value from the global state by its key.
///
/// # Arguments
///
/// * `key` - A string slice that holds the key of the value to retrieve.
///
/// # Returns
///
/// An `Option` containing an `Arc` of the value if it exists and can be downcast to the specified type, or `None` otherwise.
pub fn get_state<T: Send + Sync + 'static>(key: &str) -> Option<Arc<T>> {
    GLOBAL_STATE
        .get(key)
        .map(|v| v.clone())
        .and_then(|v| v.downcast::<T>().ok())
}
