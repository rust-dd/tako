//! `MatchedPath` extractor — the route template that matched the request.
//!
//! Returns the routing template (e.g. `/users/{id}`) instead of the concrete
//! URI (`/users/42`). Useful for metrics labels (avoiding cardinality blow-ups)
//! and structured logs.
//!
//! The router inserts `tako_rs_core::router_state::MatchedPath` into request
//! extensions during dispatch and this module re-exports the same type so the
//! extension key and the extractor are one canonical Rust type. Prior versions
//! shipped a same-named newtype here that wrapped the storage type; that made
//! it easy to insert the wrong `MatchedPath` into extensions and have lookups
//! silently miss.

pub use tako_rs_core::router_state::MatchedPath;
pub use tako_rs_core::router_state::MatchedPathMissing;
