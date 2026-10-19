//! The route template that matched, such as `/users/{id}` for `/users/42`; a bounded label
//! for metrics and logs. The same type is the extension the router inserts.

pub use tako_rs_core::router_state::MatchedPath;
pub use tako_rs_core::router_state::MatchedPathMissing;
