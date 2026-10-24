//! Process-global state with one slot per type; prefer
//! [`Router::with_state`](crate::router::Router::with_state) for router-scoped values.
//!
//! # Examples
//!
//! ```rust
//! use tako::state::{get_or_init_state, get_state};
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
//! get_or_init_state(|| config.clone());
//!
//! // Retrieve configuration by type
//! let retrieved: Option<std::sync::Arc<DatabaseConfig>> = get_state::<DatabaseConfig>();
//! assert_eq!(retrieved.as_ref().map(|c| &**c), Some(&config));
//! ```

use std::any::Any;
use std::any::TypeId;
use std::cell::RefCell;
use std::sync::Arc;
use std::sync::LazyLock;
use std::sync::OnceLock;

use scc::HashMap as SccHashMap;

type StateSlot = Arc<OnceLock<Arc<dyn Any + Send + Sync>>>;

static GLOBAL_STATE: LazyLock<SccHashMap<TypeId, StateSlot>> = LazyLock::new(SccHashMap::new);

thread_local! {
  static INITIALIZING: RefCell<Vec<TypeId>> = const { RefCell::new(Vec::new()) };
}

struct InitializationGuard;

impl InitializationGuard {
  fn enter<T: 'static>() -> Self {
    let recursive = INITIALIZING.with(|initializing| {
      let mut types = initializing.borrow_mut();
      if types.contains(&TypeId::of::<T>()) {
        return true;
      }
      types.push(TypeId::of::<T>());
      false
    });
    assert!(
      !recursive,
      "recursive state initialization for {}",
      std::any::type_name::<T>()
    );
    Self
  }
}

impl Drop for InitializationGuard {
  fn drop(&mut self) {
    INITIALIZING.with(|initializing| {
      initializing.borrow_mut().pop();
    });
  }
}

/// Returns the process-global value stored for `T`, if any.
///
/// # Examples
///
/// ```rust
/// use tako::state::{get_or_init_state, get_state};
/// use std::sync::Arc;
///
/// // Store and retrieve a string
/// get_or_init_state(|| "Hello, World!".to_string());
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
    .read_sync(&TypeId::of::<T>(), |_, slot| slot.get().cloned())
    .flatten()
    .and_then(|v| v.downcast::<T>().ok())
}

/// Returns the global `T`, first storing `init()` if the slot is empty.
///
/// `init` runs at most once, even under concurrent callers, which all get the same `Arc`.
/// A panicking initializer leaves the slot empty for a later attempt. Initializers may
/// initialize other types, but must not form dependency cycles across threads.
/// Recursive initialization of the same type on one thread panics.
pub fn get_or_init_state<T, F>(init: F) -> Arc<T>
where
  T: Send + Sync + 'static,
  F: FnOnce() -> T,
{
  let slot = GLOBAL_STATE
    .entry_sync(TypeId::of::<T>())
    .or_insert_with(|| Arc::new(OnceLock::new()))
    .clone();
  let _guard = InitializationGuard::enter::<T>();
  slot
    .get_or_init(|| Arc::new(init()))
    .clone()
    .downcast::<T>()
    .expect("get_or_init_state type mismatch — global slot already held a different type")
}
