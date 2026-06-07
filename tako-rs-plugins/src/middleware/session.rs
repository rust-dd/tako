//! Cookie-based session middleware with in-memory store.
//!
//! Provides a session mechanism using cookies and an in-memory `scc::HashMap`
//! store. Sessions are identified by a random cookie value and support
//! get / set / remove operations for arbitrary `serde`-compatible types.
//!
//! v2 additions over the original middleware:
//!
//! - **Idle vs absolute timeout.** `idle_ttl` (default 1 h) bounds inactivity;
//!   `absolute_ttl` (default 24 h) bounds total session lifetime so a stolen
//!   session id cannot be refreshed forever.
//! - **Rolling cookie refresh.** Every dirty / touched session re-emits the
//!   `Set-Cookie` header with the refreshed `Max-Age`, not just the first
//!   request.
//! - **Privilege rotation.** [`Session::rotate`] swaps the underlying session
//!   id while keeping the data — defends against fixation after login.
//! - **Bulk revocation.** [`SessionMiddleware::handle`] returns a
//!   [`SessionStoreHandle`] with a `revoke_all` API for emergency purges.
//! - **`SameSite` selection.** Default stays `Lax`, but the builder accepts
//!   `Strict` or `None` (the latter requires `Secure` per browsers).

mod cookie;
mod data;
mod layer;
mod store;

pub use cookie::SameSite;
pub use data::Session;
pub use layer::SessionMiddleware;
pub use store::SessionStoreHandle;
pub use store::SessionTtl;
