#![cfg_attr(docsrs, feature(doc_cfg))]
// Extractors keep `fn -> impl Future + Send + 'a` so their futures stay `Send` for hyper.
#![allow(clippy::manual_async_fn)]

//! Request extractors, re-exported as `tako::extractors`. `Json`, `Params` and the extractor
//! traits live in `tako-rs-core` because the router uses them.

pub mod acc_lang;
pub mod accept;
pub mod basic;
pub mod bearer;
pub mod bytes;
pub mod connect_info;
pub mod content_length_limit;
pub mod cookie_jar;
pub mod cookie_key_expansion;
pub mod cookie_private;
pub mod cookie_signed;
pub mod extension;
pub mod form;
pub mod header_map;
pub mod ipaddr;
pub mod jwt;
pub mod matched_path;
#[cfg(feature = "multipart")]
#[cfg_attr(docsrs, doc(cfg(feature = "multipart")))]
pub mod multipart;
pub mod path;
#[cfg(feature = "protobuf")]
#[cfg_attr(docsrs, doc(cfg(feature = "protobuf")))]
pub mod protobuf;
pub mod query;
pub mod query_multi;
#[cfg(any(feature = "simd-sonic", feature = "simd-json-impl"))]
#[cfg_attr(
  docsrs,
  doc(cfg(any(feature = "simd-sonic", feature = "simd-json-impl")))
)]
pub mod simdjson;
pub mod state;
#[cfg(feature = "typed-header")]
#[cfg_attr(docsrs, doc(cfg(feature = "typed-header")))]
pub mod typed_header;
pub mod uri_parts;
#[cfg(any(feature = "validator", feature = "garde"))]
#[cfg_attr(docsrs, doc(cfg(any(feature = "validator", feature = "garde"))))]
pub mod validate;
#[cfg(feature = "zero-copy-extractors")]
#[cfg_attr(docsrs, doc(cfg(feature = "zero-copy-extractors")))]
pub mod zero_copy_extractors;
