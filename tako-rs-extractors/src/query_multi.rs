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

/// Keys whose form-decoded values are split on commas before deserialization.
/// `+` becomes a space, `%2B` stays a plus, and `%2C` becomes a separator.
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
  fn rewrite<'a>(&self, query: &'a str) -> Result<Cow<'a, str>, serde_urlencoded::de::Error> {
    if self.csv_keys.is_empty() {
      return Ok(Cow::Borrowed(query));
    }

    let mut out = String::with_capacity(query.len());
    let mut first = true;
    let pairs: Vec<(String, String)> = serde_urlencoded::from_str(query)?;
    for (key, value) in pairs {
      let csv = self.csv_keys.contains(&key.as_str());
      for part in value.split(|c| csv && c == ',') {
        if !first {
          out.push('&');
        }
        first = false;
        out.push_str(&urlencoding::encode(&key));
        out.push('=');
        out.push_str(&urlencoding::encode(part));
      }
    }
    Ok(Cow::Owned(out))
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
  let rewritten = opts
    .rewrite(query)
    .map_err(|e| QueryMultiError::DeserializationError(e.to_string()))?;
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

#[cfg(test)]
mod tests {
  use serde::Deserialize;

  use super::*;

  #[derive(Debug, Deserialize, PartialEq)]
  struct Filter {
    #[serde(rename = "ta g")]
    tags: Vec<String>,
  }

  #[test]
  fn csv_decodes_form_components_before_splitting_commas() {
    let query = "ta+g=hello+world,x&ta%20g=a+b,%2B&ta+g=c%2Cd&ta%20g=%2B+%2B";
    let parsed: Filter = parse(query, &QueryMultiOptions::default().csv_key("ta g")).unwrap();
    assert_eq!(
      parsed.tags,
      ["hello world", "x", "a b", "+", "c", "d", "+ +"]
    );
  }

  #[test]
  fn ordinary_form_values_keep_commas_and_decode_plus_once() {
    let query = "ta+g=hello+world,x&ta%20g=%2B%20%2C";
    for opts in [
      QueryMultiOptions::default(),
      QueryMultiOptions::default().csv_key("other"),
    ] {
      let parsed: Filter = parse(query, &opts).unwrap();
      assert_eq!(parsed.tags, ["hello world,x", "+ ,"]);
    }
  }
}
