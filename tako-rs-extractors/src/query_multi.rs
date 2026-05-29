//! Multi-value query extractor.
//!
//! `serde_urlencoded` (the parser behind [`crate::query::Query`]) treats each
//! key as scalar, so `?tag=a&tag=b` becomes a single `tag` value (the last one
//! wins). This module exposes [`QueryMulti<T>`](crate::query_multi::QueryMulti) backed by `serde_html_form`,
//! which preserves repeated keys and decodes them into `Vec`-shaped fields.
//!
//! It also recognises CSV-style multi values inside a single key
//! (`?tags=a,b,c`) when configured via [`QueryMultiOptions::csv_key`](crate::query_multi::QueryMultiOptions::csv_key).

use std::borrow::Cow;

use http::StatusCode;
use http::request::Parts;
use serde::de::DeserializeOwned;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// Multi-value query extractor — preserves repeated keys and arrays.
///
/// # Examples
///
/// ```rust,ignore
/// use serde::Deserialize;
/// use tako::extractors::query_multi::QueryMulti;
///
/// #[derive(Deserialize)]
/// struct Filter {
///   tag: Vec<String>,
///   sort: Option<String>,
/// }
///
/// // ?tag=a&tag=b&sort=date
/// async fn handler(QueryMulti(f): QueryMulti<Filter>) -> String {
///   format!("tags={:?}, sort={:?}", f.tag, f.sort)
/// }
/// ```
pub struct QueryMulti<T>(pub T);

/// Options controlling CSV-style expansion before delegating to
/// `serde_html_form`. CSV keys are expanded so `?tags=a,b,c` becomes
/// `tags=a&tags=b&tags=c` before parsing.
#[derive(Debug, Clone, Default)]
pub struct QueryMultiOptions {
  csv_keys: Vec<&'static str>,
}

impl QueryMultiOptions {
  /// Adds a key whose CSV value should be expanded into repeated entries.
  pub fn csv_key(mut self, key: &'static str) -> Self {
    self.csv_keys.push(key);
    self
  }

  /// Internal: rewrite the query string by expanding CSV values for the
  /// configured keys. Skips keys not in `csv_keys` (passes them through).
  ///
  /// CSV detection works on the URL-decoded value so `?tags=hello%2Cworld`
  /// (a percent-encoded comma) splits the same way as `?tags=hello,world` —
  /// previously only literal commas triggered the split, which was an
  /// interop bug because the percent-encoded form is what well-behaved
  /// clients produce.
  fn rewrite<'a>(&self, query: &'a str) -> Cow<'a, str> {
    if self.csv_keys.is_empty() {
      return Cow::Borrowed(query);
    }

    let mut out = String::with_capacity(query.len());
    let mut first = true;
    for pair in query.split('&').filter(|p| !p.is_empty()) {
      let (key, value) = match pair.find('=') {
        Some(idx) => (&pair[..idx], &pair[idx + 1..]),
        None => (pair, ""),
      };
      // EXT-9: compare the *decoded* key — a client sending
      // `?ta%67s=…` (percent-encoded `g`) would otherwise bypass the
      // CSV-split rewrite because raw `ta%67s` does not equal `tags`.
      let decoded_key = urlencoding::decode(key).unwrap_or(Cow::Borrowed(key));
      let decoded_value = urlencoding::decode(value).unwrap_or(Cow::Borrowed(value));
      if self.csv_keys.contains(&decoded_key.as_ref()) && decoded_value.contains(',') {
        for part in decoded_value.split(',') {
          if !first {
            out.push('&');
          }
          first = false;
          out.push_str(key);
          out.push('=');
          // Re-encode the part so the rewritten query string remains a
          // valid `application/x-www-form-urlencoded` payload that the
          // downstream parser will decode again identically.
          out.push_str(&urlencoding::encode(part));
        }
      } else {
        if !first {
          out.push('&');
        }
        first = false;
        out.push_str(pair);
      }
    }
    Cow::Owned(out)
  }
}

/// Error returned by [`QueryMulti`].
#[derive(Debug)]
pub enum QueryMultiError {
  /// Underlying `serde_html_form` deserialization failure.
  DeserializationError(String),
}

impl Responder for QueryMultiError {
  fn into_response(self) -> tako_rs_core::types::Response {
    match self {
      Self::DeserializationError(e) => (
        StatusCode::BAD_REQUEST,
        format!("failed to deserialize query: {e}"),
      )
        .into_response(),
    }
  }
}

fn lookup_options(extensions: &http::Extensions) -> QueryMultiOptions {
  extensions
    .get::<QueryMultiOptions>()
    .cloned()
    .unwrap_or_default()
}

fn parse<T: DeserializeOwned>(query: &str, opts: &QueryMultiOptions) -> Result<T, QueryMultiError> {
  let rewritten = opts.rewrite(query);
  serde_html_form::from_str::<T>(rewritten.as_ref())
    .map_err(|e| QueryMultiError::DeserializationError(e.to_string()))
}

impl<'a, T> FromRequest<'a> for QueryMulti<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = QueryMultiError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let opts = lookup_options(req.extensions());
    let q = req.uri().query().unwrap_or("").to_string();
    futures_util::future::ready(parse::<T>(&q, &opts).map(QueryMulti))
  }
}

impl<'a, T> FromRequestParts<'a> for QueryMulti<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = QueryMultiError;

  fn from_request_parts(
    parts: &'a mut Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    let opts = lookup_options(&parts.extensions);
    let q = parts.uri.query().unwrap_or("").to_string();
    futures_util::future::ready(parse::<T>(&q, &opts).map(QueryMulti))
  }
}
