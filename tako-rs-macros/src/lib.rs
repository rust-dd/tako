//! Proc macros for the tako-rs framework.
//!
//! Provides [`route`], an attribute macro placed directly above an async
//! handler function. Given an HTTP method and a path with `{name: Type}`
//! placeholders, it generates a sibling `pub struct` whose fields exactly
//! mirror the placeholders, plus:
//!
//! - `pub const METHOD: tako::Method` and `pub const PATH: &'static str`
//! - an `impl TypedParamsStruct` that pulls each field from the request's
//!   `PathParams` extension and parses it via [`core::str::FromStr`]
//!
//! The struct name is auto-derived from the handler function's name
//! (`snake_case` → `PascalCase` + `Params`). For example, `get_user` produces
//! `GetUserParams`. Override the default with `name = "..."` if you need a
//! different identifier.
//!
//! Method-specific shortcuts ([`get`], [`post`], [`put`], [`delete`],
//! [`patch`]) take only the path and an optional `name = "..."`.
//!
//! Usage:
//!
//! ```ignore
//! use tako::{get, route};
//! use tako::extractors::typed_params::TypedParams;
//! use tako::responder::Responder;
//!
//! #[route(GET, "/users/{id: u64}/posts/{post_id: u64}")]
//! async fn get_user(TypedParams(p): TypedParams<GetUserParams>) -> impl Responder {
//!     format!("user {} post {}", p.id, p.post_id)
//! }
//!
//! #[get("/health")]
//! async fn health() -> impl Responder { "ok" }
//!
//! // …in build_router:
//! // router.route(GetUserParams::METHOD, GetUserParams::PATH, get_user);
//! // router.route(HealthParams::METHOD, HealthParams::PATH, health);
//! ```
//!
//! The macro must be attached to a free async fn at module scope — Rust
//! scopes structs declared inside fn bodies to that fn, so the generated
//! type wouldn't be reachable from the handler signature otherwise.

mod expand;
mod parse;

use proc_macro::TokenStream;
use syn::ItemFn;
use syn::parse_macro_input;

use crate::expand::expand_route;
use crate::expand::shortcut;
use crate::parse::RouteArgs;

#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
  let RouteArgs {
    method,
    path,
    name_override,
  } = parse_macro_input!(attr as RouteArgs);
  let func = parse_macro_input!(item as ItemFn);
  expand_route(method, path, name_override, func)
}

/// `#[get("/path", [name = "Foo"])]` — shorthand for `#[route(GET, ...)]`.
#[proc_macro_attribute]
pub fn get(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("GET", attr, item)
}

/// `#[post("/path", [name = "Foo"])]` — shorthand for `#[route(POST, ...)]`.
#[proc_macro_attribute]
pub fn post(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("POST", attr, item)
}

/// `#[put("/path", [name = "Foo"])]` — shorthand for `#[route(PUT, ...)]`.
#[proc_macro_attribute]
pub fn put(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("PUT", attr, item)
}

/// `#[delete("/path", [name = "Foo"])]` — shorthand for `#[route(DELETE, ...)]`.
#[proc_macro_attribute]
pub fn delete(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("DELETE", attr, item)
}

/// `#[patch("/path", [name = "Foo"])]` — shorthand for `#[route(PATCH, ...)]`.
#[proc_macro_attribute]
pub fn patch(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("PATCH", attr, item)
}
