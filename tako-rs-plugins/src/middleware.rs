//! Built-in middleware implementations.
//!
//! Each submodule provides one ready-to-use middleware. The middleware trait
//! (`Next`, `IntoMiddleware`) lives in `tako-core::middleware`.

pub mod access_log;
pub mod api_key_auth;
pub mod basic_auth;
pub mod bearer_auth;
pub mod body_limit;
pub mod circuit_breaker;
pub mod csrf;
pub mod etag;
pub mod healthcheck;
#[cfg(feature = "hmac-signature")]
#[cfg_attr(docsrs, doc(cfg(feature = "hmac-signature")))]
pub mod hmac_signature;
#[cfg(feature = "ip-filter")]
#[cfg_attr(docsrs, doc(cfg(feature = "ip-filter")))]
pub mod ip_filter;
#[cfg(feature = "json-schema")]
#[cfg_attr(docsrs, doc(cfg(feature = "json-schema")))]
pub mod json_schema;
pub mod jwt_auth;
pub mod problem_json;
pub mod request_id;
pub mod security_headers;
pub mod session;
pub mod tenant;
pub mod timeout;
pub mod traceparent;
pub mod upload_progress;
