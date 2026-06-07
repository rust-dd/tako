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

/// After the path literal there can optionally be `, name = "Foo"`. Returns
/// `Ok(None)` if the comma/keyword is absent, `Err` only on a malformed key.
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

/// Parses path placeholders. Two syntaxes are accepted:
/// - typed: `{id: u64}` — emits a field on the generated `*Params` struct
/// - untyped: `{id}` — matchit/axum-style; passes through untouched and does
///   not contribute to the `*Params` struct
///
/// Returns the matchit-friendly stripped path (every placeholder reduced to
/// `{name}`) plus the list of typed `(name, type)` pairs only.
pub(crate) fn parse_path(path: &str, span: Span) -> syn::Result<(String, Vec<PathParam>)> {
  // Route paths are ASCII per RFC 3986 (`reserved` + `unreserved` are both
  // ASCII subsets). Reject anything else up front rather than mojibake the
  // byte stream into the stripped output: previously a multi-byte UTF-8
  // char like `é` (`0xC3 0xA9`) was pushed as two distinct `char` values,
  // both Latin-1 codepoints, breaking exact-path matching against the
  // matchit-compiled route.
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
      // Stray `}` without a preceding `{` is a path-syntax mistake. Reject
      // explicitly so the error surfaces at macro-expansion time rather
      // than as a downstream matchit mismatch.
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
    if let Some((name_str, ty_str)) = inner.split_once(':') {
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
