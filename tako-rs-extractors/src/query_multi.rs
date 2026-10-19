//! Query strings parsed with `serde_html_form`, so repeated keys fill `Vec` fields.

use std::borrow::Cow;

use http::StatusCode;
use http::request::Parts;
use serde::de::DeserializeOwned;
use tako_rs_core::extractors::FromRequest;
use tako_rs_core::extractors::FromRequestParts;
use tako_rs_core::responder::Responder;
use tako_rs_core::types::Request;

/// The query string as `T`, with the CSV splitting of any [`QueryMultiOptions`] in the
/// request extensions.
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

/// Keys whose comma-separated values are split into repeated keys before parsing.
#[derive(Debug, Clone, Default)]
pub struct QueryMultiOptions {
  csv_keys: Vec<&'static str>,
}

impl QueryMultiOptions {
  /// Splits `key=a,b` into `key=a&key=b`.
  pub fn csv_key(mut self, key: &'static str) -> Self {
    self.csv_keys.push(key);
    self
  }

  /// Compares decoded keys and splits decoded values, so `ta%67s` matches `tags` and `%2C`
  /// splits like `,`.
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
          // Re-encoded so the parser decodes it back to the same value.
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

/// Rejection of [`QueryMulti`]; responds 400.
#[derive(Debug)]
pub enum QueryMultiError {
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
