//! The `Params<T>` extractor and the `PathParams` request-extension carrier.

use serde::de::DeserializeOwned;
use smallvec::SmallVec;

use super::deserializer::PathParamsDeserializer;
use super::error::ParamsError;
use crate::extractors::FromRequest;
use crate::extractors::FromRequestParts;
use crate::types::Request;

/// Internal helper struct for storing path parameters extracted from routes.
#[derive(Clone, Default)]
#[doc(hidden)]
pub struct PathParams(pub SmallVec<[(String, String); 4]>);

/// Path parameter extractor with automatic deserialization to typed structures.
#[doc(alias = "params")]
pub struct Params<T>(pub T);

impl<'a, T> FromRequest<'a> for Params<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = ParamsError;

  fn from_request(
    req: &'a mut Request,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_params(req.extensions()))
  }
}

impl<'a, T> FromRequestParts<'a> for Params<T>
where
  T: DeserializeOwned + Send + 'a,
{
  type Error = ParamsError;

  fn from_request_parts(
    parts: &'a mut http::request::Parts,
  ) -> impl core::future::Future<Output = core::result::Result<Self, Self::Error>> + Send + 'a {
    futures_util::future::ready(Self::extract_params(&parts.extensions))
  }
}

impl<T> Params<T>
where
  T: DeserializeOwned,
{
  /// Extracts and deserializes path parameters from request extensions.
  fn extract_params(extensions: &http::Extensions) -> Result<Params<T>, ParamsError> {
    let path_params = extensions
      .get::<PathParams>()
      .ok_or(ParamsError::MissingPathParams)?;

    let parsed = T::deserialize(PathParamsDeserializer(&path_params.0))
      .map_err(|e| ParamsError::DeserializationError(e.to_string()))?;

    Ok(Params(parsed))
  }
}

#[cfg(test)]
mod tests {
  use serde::Deserialize;

  use super::*;

  #[derive(Debug, Deserialize, PartialEq)]
  struct UserParams {
    id: u64,
    name: String,
  }

  #[test]
  fn extract_params_returns_missing_when_extension_absent() {
    let extensions = http::Extensions::new();
    match Params::<UserParams>::extract_params(&extensions) {
      Err(e) => assert_eq!(e, ParamsError::MissingPathParams),
      Ok(_) => panic!("expected MissingPathParams"),
    }
  }

  #[test]
  fn extract_params_returns_value_when_extension_present() {
    let mut extensions = http::Extensions::new();
    let mut params = SmallVec::<[(String, String); 4]>::new();
    params.push(("id".to_string(), "5".to_string()));
    params.push(("name".to_string(), "bob".to_string()));
    extensions.insert(PathParams(params));

    let extracted = Params::<UserParams>::extract_params(&extensions).expect("extract ok");
    assert_eq!(
      extracted.0,
      UserParams {
        id: 5,
        name: "bob".to_string(),
      }
    );
  }
}
