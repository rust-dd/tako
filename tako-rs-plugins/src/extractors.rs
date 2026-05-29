//! Plugin/middleware-coupled extractors.
//!
//! These extractors only make sense when paired with a specific middleware
//! that pre-populates request extensions. Putting them next to the producing
//! middleware (instead of in `tako-extractors`) keeps the trust boundary
//! visible: a verified-claims extractor only works after the matching auth
//! middleware ran on the same request.

pub mod jwt;
