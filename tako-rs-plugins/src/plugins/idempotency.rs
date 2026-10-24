//! `Idempotency-Key` de-duplication: repeats replay the first response, concurrent duplicates
//! wait for it, and reusing a key with a different payload gets 409. Bodies are buffered.
//!
//! Keyed requests require a trusted [`IdempotencyIdentity`] extension by default. Run
//! authentication and authorization before this plugin, including on retries. Use
//! [`IdempotencyBuilder::identity_fn`] for application identity types, or explicitly opt into
//! [`IdempotencyBuilder::anonymous`] only for public operations with shareable responses.

mod config;
mod identity;
mod plugin;
mod response;
mod store;

pub use config::Config;
pub use config::IdempotencyBuilder;
pub use config::Scope;
pub use identity::IdempotencyIdentity;
pub use plugin::IdempotencyPlugin;
