//! CSRF protection middleware.
//!
//! Default mode is the **double-submit cookie** pattern: a random token is
//! placed in a cookie *and* must be echoed back in a request header (or form
//! field). The middleware verifies the two values match and that, when a
//! [`Session`](crate::middleware::session::Session) extension is present, the cookie was issued for the current
//! session.
//!
//! v2 additions:
//!
//! - **Session-bound tokens.** When a [`Session`](crate::middleware::session::Session) extension is in scope, the
//!   token is stored in the session and the cookie value must agree with it.
//!   Tokens carried over from a previous session id (after privilege rotation)
//!   are rejected.
//! - **Origin / Referer fallback.** When neither cookie nor header is set
//!   (legacy clients) the middleware can fall back to a strict
//!   `Origin` / `Referer` allow-list before rejecting.
//! - **Configurable `SameSite`.** Defaults stay `Strict`. Choose `Lax` if
//!   the application embeds the API in a same-site form post flow.

mod config;
mod cookie;
mod layer;
mod token;

pub use config::Csrf;
