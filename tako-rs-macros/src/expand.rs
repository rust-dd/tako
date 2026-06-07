use proc_macro::TokenStream;
use proc_macro2::Span;
use proc_macro2::TokenStream as TokenStream2;
use quote::format_ident;
use quote::quote;
use syn::Ident;
use syn::ItemFn;
use syn::LitStr;
use syn::Type;
use syn::parse_macro_input;

use crate::parse::ShortcutArgs;
use crate::parse::parse_path;

/// `snake_case` → `PascalCase`. `get_user` → `GetUser`. ASCII only, which is
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

/// Shared expansion: given a method ident, a path literal, an optional struct
/// name override, and the handler fn, produce the generated tokens.
///
/// Only emits the `*Params` struct when the path contains at least one typed
/// placeholder (`{id: u64}`). Pure-static or untyped-only paths skip the
/// struct entirely and just register the route.
pub(crate) fn expand_route(
  method: Ident,
  path: LitStr,
  name_override: Option<Ident>,
  func: ItemFn,
) -> TokenStream {
  let span = path.span();
  let path_str = path.value();
  let (stripped, params) = match parse_path(&path_str, span) {
    Ok(v) => v,
    Err(e) => return e.to_compile_error().into(),
  };

  let fn_name = &func.sig.ident;
  // Append a short fingerprint of (method + path) so two handlers that
  // happen to share the same function identifier — common when several
  // modules each define an `fn handler` — generate distinct linkme
  // registrars. Without the suffix the second module's static silently
  // overwrote the first at link time.
  let registrar_suffix = {
    let key = format!("{method}_{path_str}");
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325; // FNV-1a 64-bit offset basis
    for byte in key.as_bytes() {
      hash ^= u64::from(*byte);
      hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{hash:016X}")
  };
  let registrar_ident = format_ident!(
    "__TAKO_REGISTER_{}_{}",
    fn_name.to_string().to_uppercase(),
    registrar_suffix,
    span = fn_name.span()
  );

  // No typed placeholders.
  if params.is_empty() {
    // Explicit `name = "..."` keeps emitting a unit marker struct so callers
    // can still reference `Name::METHOD` / `Name::PATH`. Without an override
    // we skip the struct entirely.
    if let Some(struct_name) = name_override {
      let expanded: TokenStream2 = quote! {
        pub struct #struct_name;

        impl #struct_name {
          pub const METHOD: ::tako::Method = ::tako::Method::#method;
          pub const PATH: &'static str = #stripped;
        }

        #[::tako::__private::linkme::distributed_slice(::tako::router::TAKO_ROUTES)]
        #[linkme(crate = ::tako::__private::linkme)]
        static #registrar_ident: fn(&mut ::tako::router::Router) = |__router| {
          __router.route(#struct_name::METHOD, #struct_name::PATH, #fn_name);
        };

        #func
      };
      return expanded.into();
    }

    let expanded: TokenStream2 = quote! {
      #[::tako::__private::linkme::distributed_slice(::tako::router::TAKO_ROUTES)]
      #[linkme(crate = ::tako::__private::linkme)]
      static #registrar_ident: fn(&mut ::tako::router::Router) = |__router| {
        __router.route(::tako::Method::#method, #stripped, #fn_name);
      };

      #func
    };
    return expanded.into();
  }

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

  let expanded: TokenStream2 = quote! {
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

    #[::tako::__private::linkme::distributed_slice(::tako::router::TAKO_ROUTES)]
    #[linkme(crate = ::tako::__private::linkme)]
    static #registrar_ident: fn(&mut ::tako::router::Router) = |__router| {
      __router.route(#struct_name::METHOD, #struct_name::PATH, #fn_name);
    };

    #func
  };

  expanded.into()
}

/// Common driver for the method shortcuts (`#[get]`, `#[post]`, ...).
pub(crate) fn shortcut(
  method_name: &'static str,
  attr: TokenStream,
  item: TokenStream,
) -> TokenStream {
  let ShortcutArgs {
    path,
    name_override,
  } = parse_macro_input!(attr as ShortcutArgs);
  let func = parse_macro_input!(item as ItemFn);
  let method = Ident::new(method_name, Span::call_site());
  expand_route(method, path, name_override, func)
}
