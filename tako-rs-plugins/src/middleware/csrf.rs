//! Double-submit-cookie CSRF protection bound to the [`Session`](super::session::Session), so a
//! rotated session invalidates its token; trusted origins are the fallback check.

mod config;
mod cookie;
mod layer;
mod stored;
mod token;

pub use config::Csrf;
