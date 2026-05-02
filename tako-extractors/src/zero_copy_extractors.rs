//! Zero-copy extractor variants.
//!
//! These types live behind the `zero-copy-extractors` cargo feature. They
//! mirror the regular extractors (`Json`, `Form`, `Query`, `HeaderMap`,
//! `Path`) but expose borrowed views into the request so deserialization can
//! avoid per-field allocations.
//!
//! - `BytesBorrowed` / `BodySliceBorrowed` — collect the body once and lend
//!   `&'a Bytes` / `&'a [u8]` to subsequent extractors.
//! - `JsonBorrowed<T>` — `serde::Deserialize<'a>` JSON read out of the cached body.
//! - `FormBorrowed<T>` — `application/x-www-form-urlencoded` parse out of the cached body.
//! - `RawQueryBorrowed` / `QueryBorrowed<T>` — borrow directly from the URI query slice.
//! - `PathBorrowed` — borrow the URI path slice.
//! - `HeaderMapBorrowed` — borrow the full `&HeaderMap`.
//! - `AuthorizationBorrowed` / `AuthorizationOptBorrowed` — borrow the `Authorization` header.

pub mod bytes;
pub mod form;
pub mod header;
pub mod header_map;
pub mod json;
pub mod path;
pub mod query;
