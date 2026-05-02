#![cfg_attr(docsrs, feature(doc_cfg))]

//! Concrete request extractor implementations for the Tako framework.
//!
//! The `FromRequest` and `FromRequestParts` traits live in `tako-core`. The
//! `Json` and `Params` extractors also stay there because their internal types
//! are referenced by the router. Everything else (header_map, cookies, query,
//! path, form, ipaddr, accept, basic/bearer auth, jwt, byte body, range,
//! state, plus the optional multipart/protobuf/simdjson and zero-copy variants)
//! lives here. Re-exported under `tako::extractors::*` via the umbrella crate.

/// Accept-Language header parsing and locale extraction.
pub mod acc_lang;

/// Content negotiation via Accept header parsing.
pub mod accept;

/// Basic HTTP authentication credential extraction.
pub mod basic;

/// Bearer token authentication extraction from Authorization header.
pub mod bearer;

/// Raw byte data extraction from request bodies.
pub mod bytes;

/// `ConnectInfo<T>` typed view over `tako_core::conn_info::ConnInfo`.
pub mod connect_info;

/// `ContentLengthLimit<T, N>` body-bound extractor wrapper.
pub mod content_length_limit;

/// `Extension<T>` typed extractor for request-scoped values.
pub mod extension;

/// `MatchedPath` extractor — the route template that matched the request.
pub mod matched_path;

/// URI-derived extractors (`OriginalUri`, `Host`, `Scheme`).
pub mod uri_parts;

/// `TypedHeader<H>` strongly-typed header extractor (requires `typed-header` feature).
#[cfg(feature = "typed-header")]
#[cfg_attr(docsrs, doc(cfg(feature = "typed-header")))]
pub mod typed_header;

/// Cookie parsing and management utilities.
pub mod cookie_jar;

/// Cookie key derivation and expansion for encryption/signing.
pub mod cookie_key_expansion;

/// Private (encrypted) cookie handling with automatic decryption.
pub mod cookie_private;

/// Signed cookie handling with HMAC verification.
pub mod cookie_signed;

/// Form data (application/x-www-form-urlencoded) parsing.
pub mod form;

/// HTTP header map extraction and manipulation.
pub mod header_map;

/// IP address extraction from request headers and connection info.
pub mod ipaddr;

/// JSON Web Token (JWT) handling with HMAC verification.
pub mod jwt;

/// URL path component extraction and manipulation.
pub mod path;

/// Query parameter parsing from URL query strings.
pub mod query;

/// Multi-value query parser preserving repeated keys and CSV expansions.
pub mod query_multi;

/// `Validated<T>` wrapper that runs `validator` / `garde` rules after extraction.
#[cfg(any(feature = "validator", feature = "garde"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "validator", feature = "garde"))))]
pub mod validate;

/// Global state extraction for accessing shared app state.
pub mod state;

/// Multipart form data parsing for file uploads and complex forms.
#[cfg(feature = "multipart")]
#[cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
pub mod multipart;

/// Protobuf request body parsing and deserialization.
#[cfg(feature = "protobuf")]
#[cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
pub mod protobuf;

/// High-performance JSON parsing using SIMD acceleration.
#[cfg(feature = "simd")]
#[cfg_attr(docsrs, doc(cfg(feature = "simd")))]
pub mod simdjson;

/// Zero-copy extraction helpers.
#[cfg(feature = "zero-copy-extractors")]
#[cfg_attr(docsrs, doc(cfg(feature = "zero-copy-extractors")))]
pub mod zero_copy_extractors;
