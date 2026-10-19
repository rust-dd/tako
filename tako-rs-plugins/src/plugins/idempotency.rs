//! `Idempotency-Key` de-duplication: repeats replay the first response, concurrent duplicates
//! wait for it, and reusing a key with a different payload gets 409. Bodies are buffered.

mod config;
mod plugin;
mod response;
mod store;

pub use config::Config;
pub use config::IdempotencyBuilder;
pub use config::Scope;
pub use plugin::IdempotencyPlugin;
