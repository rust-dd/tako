//! The `#[route]` attribute and its method shortcuts, re-exported by `tako`.

mod expand;
mod parse;

use proc_macro::TokenStream;
use syn::ItemFn;
use syn::parse_macro_input;

use crate::expand::expand_route;
use crate::expand::shortcut;
use crate::parse::RouteArgs;

/// Registers the handler for `Router::mount_all`; typed placeholders such as `{id: u64}` also
/// generate a `TypedParams` struct named `<Handler>Params` unless `name = "..."` overrides it.
///
/// ```ignore
/// use tako::{get, route};
/// use tako::extractors::typed_params::TypedParams;
/// use tako::responder::Responder;
///
/// #[route(GET, "/users/{id: u64}/posts/{post_id: u64}")]
/// async fn get_user(TypedParams(p): TypedParams<GetUserParams>) -> impl Responder {
///     format!("user {} post {}", p.id, p.post_id)
/// }
///
/// #[get("/health")]
/// async fn health() -> impl Responder { "ok" }
///
/// // …in build_router, one route at a time or every attribute route at once:
/// // router.route(GetUserParams::METHOD, GetUserParams::PATH, get_user);
/// // router.mount_all();
/// ```
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

/// `#[head("/path", [name = "Foo"])]` — shorthand for `#[route(HEAD, ...)]`.
#[proc_macro_attribute]
pub fn head(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("HEAD", attr, item)
}

/// `#[options("/path", [name = "Foo"])]` — shorthand for `#[route(OPTIONS, ...)]`.
#[proc_macro_attribute]
pub fn options(attr: TokenStream, item: TokenStream) -> TokenStream {
  shortcut("OPTIONS", attr, item)
}
