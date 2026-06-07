//! Signed cookie extraction and management for HTTP requests.
//!
//! This module provides the [`CookieSigned`](crate::cookie_signed::CookieSigned) extractor that manages HMAC-signed cookies
//! using a master key. Signed cookies use HMAC (Hash-based Message Authentication Code)
//! to ensure that cookie values haven't been tampered with, while keeping the content
//! readable. This provides integrity protection without confidentiality.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::cookie_signed::CookieSigned;
//! use cookie::{Cookie, Key};
//!
//! async fn handle_signed_cookies(mut signed: CookieSigned) {
//!     // Add a signed cookie
//!     signed.add(Cookie::new("user_id", "12345"));
//!
//!     // Retrieve and verify a cookie
//!     if let Some(user_id) = signed.get_value("user_id") {
//!         println!("User ID: {}", user_id);
//!     }
//!
//!     // Check if a cookie exists and has valid signature
//!     if signed.verify("session_token") {
//!         println!("Valid session found");
//!     }
//! }
//! ```

mod error;
mod extract;
mod jar;
mod key;

pub use error::CookieSignedError;
pub use jar::CookieSigned;
pub use key::KeyRing;
