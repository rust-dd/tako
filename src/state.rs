use std::{any::Any, sync::Arc};

use dashmap::DashMap;
use once_cell::sync::Lazy;

pub(crate) static GLOBAL_STATE: Lazy<DashMap<String, Arc<dyn Any + Send + Sync>>> =
    Lazy::new(|| DashMap::new());

pub fn set_state<T: Send + Sync + 'static>(key: &str, value: T) {
    GLOBAL_STATE.insert(key.to_string(), Arc::new(value));
}

pub fn get_state<T: Send + Sync + 'static>(key: &str) -> Option<Arc<T>> {
    GLOBAL_STATE
        .get(key)
        .map(|v| v.clone())
        .and_then(|v| v.downcast::<T>().ok())
}
