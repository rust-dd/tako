#![cfg_attr(docsrs, feature(doc_cfg))]

//! Built-in middleware and plugins; the `tako-rs` umbrella crate re-exports them as
//! `tako::middleware` and `tako::plugins`.

pub mod extractors;
pub mod middleware;
pub mod plugins;
pub mod stores;
