//! Borrowing variants of the body, JSON, form, query, path and header extractors. The body
//! ones collect the body once into the request extensions and borrow from it.

pub mod bytes;
pub mod form;
pub mod header;
pub mod header_map;
pub mod json;
pub mod path;
pub mod query;
