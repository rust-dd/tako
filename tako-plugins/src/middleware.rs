//! Built-in middleware implementations.
//!
//! Each submodule provides one ready-to-use middleware. The middleware trait
//! (`Next`, `IntoMiddleware`) lives in `tako-core::middleware`.

pub mod api_key_auth;
pub mod basic_auth;
pub mod bearer_auth;
pub mod body_limit;
pub mod csrf;
pub mod jwt_auth;
pub mod request_id;
pub mod security_headers;
pub mod session;
pub mod upload_progress;
