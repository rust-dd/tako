//! Cookie sessions with idle and absolute timeouts, rolling refresh, id rotation against
//! fixation and bulk revocation; the in-memory store is the default backend.

mod cookie;
mod data;
mod layer;
mod store;

pub use cookie::SameSite;
pub(crate) use cookie::extract_cookie_value;
pub use data::Session;
pub use layer::SessionMiddleware;
pub use store::SessionStoreHandle;
pub use store::SessionTtl;
