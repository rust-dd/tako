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
//! (snake_case → PascalCase + `Params`). For example, `get_user` produces
//! `GetUserParams`. Override the default with `name = "..."` if you need a
//! different identifier.
//!
//! Usage:
//!
//! ```ignore
//! use tako::route;
//! use tako::extractors::typed_params::TypedParams;
//! use tako::responder::Responder;
//!
//! #[route(GET, "/users/{id: u64}/posts/{post_id: u64}")]
//! async fn get_user(TypedParams(p): TypedParams<GetUserParams>) -> impl Responder {
//!     format!("user {} post {}", p.id, p.post_id)
//! }
//!
//! // …in build_router:
//! // router.route(GetUserParams::METHOD, GetUserParams::PATH, get_user);
//! ```
//!
//! The macro must be attached to a free async fn at module scope — Rust
//! scopes structs declared inside fn bodies to that fn, so the generated
//! type wouldn't be reachable from the handler signature otherwise.

use proc_macro::TokenStream;
use proc_macro2::Span;
use quote::{format_ident, quote};
use syn::parse::{Parse, ParseStream};
use syn::{Ident, ItemFn, LitStr, Token, Type, parse_macro_input, parse_str};

struct RouteArgs {
  method: Ident,
  path: LitStr,
  name_override: Option<Ident>,
}

impl Parse for RouteArgs {
  fn parse(input: ParseStream) -> syn::Result<Self> {
    let method: Ident = input.parse()?;
    input.parse::<Token![,]>()?;
    let path: LitStr = input.parse()?;
    let mut name_override = None;
    if input.parse::<Token![,]>().is_ok() && !input.is_empty() {
      let key: Ident = input.parse()?;
      if key != "name" {
        return Err(syn::Error::new(key.span(), "expected `name = \"...\"`"));
      }
      input.parse::<Token![=]>()?;
      let lit: LitStr = input.parse()?;
      let ident: Ident = parse_str(&lit.value())
        .map_err(|e| syn::Error::new(lit.span(), format!("invalid struct name: {e}")))?;
      name_override = Some(ident);
    }
    Ok(Self {
      method,
      path,
      name_override,
    })
  }
}

struct PathParam {
  name: Ident,
  ty: Type,
}

/// Parses `"/users/{id: u64}/posts/{post_id: u64}"` into the matchit-friendly
/// stripped path `"/users/{id}/posts/{post_id}"` plus a list of `(name, type)`
/// pairs.
fn parse_path(path: &str, span: Span) -> syn::Result<(String, Vec<PathParam>)> {
  let mut stripped = String::with_capacity(path.len());
  let mut params = Vec::new();
  let bytes = path.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    let c = bytes[i];
    if c != b'{' {
      stripped.push(c as char);
      i += 1;
      continue;
    }
    let close = (i + 1..bytes.len())
      .find(|&j| bytes[j] == b'}')
      .ok_or_else(|| syn::Error::new(span, "unclosed '{' in path"))?;
    let inner = &path[i + 1..close];
    let (name_str, ty_str) = inner.split_once(':').ok_or_else(|| {
      syn::Error::new(
        span,
        format!("placeholder '{{{inner}}}' must be 'name: Type'"),
      )
    })?;
    let name: Ident = parse_str(name_str.trim()).map_err(|e| {
      syn::Error::new(
        span,
        format!("invalid placeholder name '{}': {e}", name_str.trim()),
      )
    })?;
    let ty: Type = parse_str(ty_str.trim()).map_err(|e| {
      syn::Error::new(
        span,
        format!("invalid placeholder type '{}': {e}", ty_str.trim()),
      )
    })?;
    stripped.push('{');
    stripped.push_str(&name.to_string());
    stripped.push('}');
    params.push(PathParam { name, ty });
    i = close + 1;
  }
  Ok((stripped, params))
}

/// snake_case → PascalCase. `get_user` → `GetUser`. ASCII only, which is
/// fine for Rust identifiers.
fn pascal_case(s: &str) -> String {
  let mut out = String::with_capacity(s.len());
  let mut next_upper = true;
  for ch in s.chars() {
    if ch == '_' {
      next_upper = true;
    } else if next_upper {
      out.extend(ch.to_uppercase());
      next_upper = false;
    } else {
      out.push(ch);
    }
  }
  out
}

#[proc_macro_attribute]
pub fn route(attr: TokenStream, item: TokenStream) -> TokenStream {
  let RouteArgs {
    method,
    path,
    name_override,
  } = parse_macro_input!(attr as RouteArgs);
  let func = parse_macro_input!(item as ItemFn);

  let span = path.span();
  let path_str = path.value();
  let (stripped, params) = match parse_path(&path_str, span) {
    Ok(v) => v,
    Err(e) => return e.to_compile_error().into(),
  };

  let fn_name = &func.sig.ident;
  let struct_name = name_override.unwrap_or_else(|| {
    format_ident!(
      "{}Params",
      pascal_case(&fn_name.to_string()),
      span = fn_name.span()
    )
  });

  let field_idents: Vec<&Ident> = params.iter().map(|p| &p.name).collect();
  let field_names_str: Vec<String> = params.iter().map(|p| p.name.to_string()).collect();
  let field_types: Vec<&Type> = params.iter().map(|p| &p.ty).collect();

  let expanded = quote! {
    pub struct #struct_name {
      #(pub #field_idents: #field_types,)*
    }

    impl #struct_name {
      pub const METHOD: ::tako::Method = ::tako::Method::#method;
      pub const PATH: &'static str = #stripped;
    }

    impl ::tako::extractors::typed_params::TypedParamsStruct for #struct_name {
      fn from_path_params(
        __pp: &::tako::extractors::params::PathParams,
      ) -> ::core::result::Result<Self, ::tako::extractors::typed_params::TypedParamsError> {
        ::core::result::Result::Ok(Self {
          #(
            #field_idents: {
              let __raw = __pp
                .0
                .iter()
                .find(|(__k, _)| __k.as_str() == #field_names_str)
                .map(|(_, __v)| __v.as_str())
                .ok_or(::tako::extractors::typed_params::TypedParamsError::MissingField(
                  #field_names_str,
                ))?;
              <#field_types as ::core::str::FromStr>::from_str(__raw).map_err(|__e| {
                ::tako::extractors::typed_params::TypedParamsError::Parse(
                  #field_names_str,
                  __e.to_string(),
                )
              })?
            },
          )*
        })
      }
    }

    #func
  };

  expanded.into()
}
