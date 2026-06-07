//! Path parameter extraction and deserialization for dynamic route segments.
//!
//! This module provides extractors for parsing path parameters from dynamic route segments
//! into strongly-typed Rust structures. It handles parameter extraction from routes like
//! `/users/{id}` or `/posts/{post_id}/comments/{comment_id}` and automatically deserializes
//! them using serde. The extractor supports type coercion for common types like integers,
//! floats, and strings, making it easy to work with typed path parameters in handlers.
//!
//! # Examples
//!
//! ```rust
//! use tako::extractors::params::Params;
//! use tako::extractors::FromRequest;
//! use tako::types::Request;
//! use serde::Deserialize;
//!
//! #[derive(Debug, Deserialize)]
//! struct UserParams {
//!     id: u64,
//!     name: String,
//! }
//!
//! // For route: /users/{id}/profile/{name}
//! async fn user_profile(mut req: Request) -> Result<String, Box<dyn std::error::Error>> {
//!     let params: Params<UserParams> = Params::from_request(&mut req).await?;
//!
//!     Ok(format!("User ID: {}, Name: {}", params.0.id, params.0.name))
//! }
//!
//! // Simple single parameter extraction
//! #[derive(Deserialize)]
//! struct IdParam {
//!     id: u32,
//! }
//!
//! async fn get_item(params: Params<IdParam>) -> String {
//!     format!("Item ID: {}", params.0.id)
//! }
//! ```

mod decode;
mod deserializer;
mod error;
mod extractor;

pub use error::ParamsError;
pub use extractor::Params;
pub use extractor::PathParams;
