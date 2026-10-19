//! HMAC-signed cookies: clients can read them, but tampering is detected. The extractor takes
//! a [`KeyRing`], or else a `cookie::Key`, from the request extensions.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_signed::CookieSigned;
//! use cookie::{Cookie, Key};
//!
//! async fn handle_signed_cookies(mut signed: CookieSigned) -> CookieSigned {
//!     // Add a signed cookie
//!     signed.add(Cookie::new("user_id", "12345"));
//!
//!     // Retrieve and verify a cookie
//!     if let Some(user_id) = signed.get_value("user_id") {
//!         println!("User ID: {}", user_id);
//!     }
//!
//!     // Check if a cookie exists and has valid signature
//!     if signed.contains("session_token") {
//!         println!("Valid session found");
//!     }
//!     signed
//! }
//! ```

mod error;
mod extract;
mod jar;
mod key;

pub use error::CookieSignedError;
pub use jar::CookieSigned;
pub use key::KeyRing;
