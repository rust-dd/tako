use proc_macro2::Span;
use syn::Ident;
use syn::LitStr;
use syn::Token;
use syn::Type;
use syn::parse::Parse;
use syn::parse::ParseStream;
use syn::parse_str;

pub(crate) struct RouteArgs {
  pub(crate) method: Ident,
  pub(crate) path: LitStr,
  pub(crate) name_override: Option<Ident>,
}

impl Parse for RouteArgs {
  fn parse(input: ParseStream) -> syn::Result<Self> {
    let method: Ident = input.parse()?;
    input.parse::<Token![,]>()?;
    let path: LitStr = input.parse()?;
    let name_override = parse_optional_name(input)?;
    Ok(Self {
      method,
      path,
      name_override,
    })
  }
}

pub(crate) struct ShortcutArgs {
  pub(crate) path: LitStr,
  pub(crate) name_override: Option<Ident>,
}

impl Parse for ShortcutArgs {
  fn parse(input: ParseStream) -> syn::Result<Self> {
    let path: LitStr = input.parse()?;
    let name_override = parse_optional_name(input)?;
    Ok(Self {
      path,
      name_override,
    })
  }
}

/// The optional `, name = "Foo"` after the path; a trailing comma alone is accepted.
fn parse_optional_name(input: ParseStream) -> syn::Result<Option<Ident>> {
  if input.is_empty() {
    return Ok(None);
  }
  input.parse::<Token![,]>()?;
  if input.is_empty() {
    return Ok(None);
  }
  let key: Ident = input.parse()?;
  if key != "name" {
    return Err(syn::Error::new(key.span(), "expected `name = \"...\"`"));
  }
  input.parse::<Token![=]>()?;
  let lit: LitStr = input.parse()?;
  let ident: Ident = parse_str(&lit.value())
    .map_err(|e| syn::Error::new(lit.span(), format!("invalid struct name: {e}")))?;
  Ok(Some(ident))
}

pub(crate) struct PathParam {
  pub(crate) name: Ident,
  pub(crate) ty: Type,
}

/// Reduces every placeholder to `{name}` for matchit and returns the typed `{name: Type}` ones;
/// untyped `{name}` and terminal `{*name}` captures add no field.
pub(crate) fn parse_path(path: &str, span: Span) -> syn::Result<(String, Vec<PathParam>)> {
  // RFC 3986 paths are ASCII, and the byte loop below would split multi-byte characters.
  if !path.is_ascii() {
    return Err(syn::Error::new(
      span,
      "route path must be ASCII (RFC 3986); percent-encode any non-ASCII characters",
    ));
  }
  let mut stripped = String::with_capacity(path.len());
  let mut typed = Vec::new();
  let bytes = path.as_bytes();
  let mut i = 0;
  while i < bytes.len() {
    let c = bytes[i];
    if c == b'}' {
      return Err(syn::Error::new(
        span,
        "unexpected '}' in path (no matching '{')",
      ));
    }
    if c != b'{' {
      stripped.push(c as char);
      i += 1;
      continue;
    }
    let close = (i + 1..bytes.len())
      .find(|&j| bytes[j] == b'}')
      .ok_or_else(|| syn::Error::new(span, "unclosed '{' in path"))?;
    let inner = &path[i + 1..close];
    if let Some(name_str) = inner.trim().strip_prefix('*') {
      if i == 0 || bytes[i - 1] != b'/' || close + 1 != bytes.len() {
        return Err(syn::Error::new(
          span,
          "catch-all must be the final path segment",
        ));
      }
      let name: Ident = parse_str(name_str.trim()).map_err(|_| {
        syn::Error::new(
          span,
          "catch-all must contain an untyped identifier: {*name}",
        )
      })?;
      stripped.push_str("{*");
      stripped.push_str(&name.to_string());
      stripped.push('}');
    } else if let Some((name_str, ty_str)) = inner.split_once(':') {
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
      typed.push(PathParam { name, ty });
    } else {
      let name: Ident = parse_str(inner.trim()).map_err(|e| {
        syn::Error::new(
          span,
          format!("invalid placeholder name '{}': {e}", inner.trim()),
        )
      })?;
      stripped.push('{');
      stripped.push_str(&name.to_string());
      stripped.push('}');
    }
    i = close + 1;
  }
  Ok((stripped, typed))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn catch_all_is_preserved_without_a_typed_field() {
    let (path, params) = parse_path("/files/{*rest}", Span::call_site()).unwrap();
    assert_eq!(path, "/files/{*rest}");
    assert!(params.is_empty());
  }

  #[test]
  fn malformed_catch_all_has_a_specific_error() {
    for path in [
      "/files/{*}",
      "/files/{**rest}",
      "/files/{*rest: String}",
      "/files/{*rest}/more",
    ] {
      let Err(error) = parse_path(path, Span::call_site()) else {
        panic!("accepted invalid catch-all: {path}");
      };
      assert!(error.to_string().contains("catch-all"), "{path}: {error}");
    }
  }
}
